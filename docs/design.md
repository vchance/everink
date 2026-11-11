# High-level design

Status: proposal, 2026-11-11. Not yet accepted by the owner. It turns `vision.md` and the owner's product
description (below) into a system shape that later documents break down part by part. Numbers in it are starting
values to test, not measurements.

## What the product does

The owner's description (2026-11-11), restated as requirements:

1. **One surface, many users.** Several people use the same surface at the same time.
2. **Draw anything.** Freehand ink and any other graphics (shapes, lines, paths).
3. **Place pre-drawn objects** from a library, and move them around.
4. **Text** that can be placed and moved like any other object.
5. **Paste images**, and possibly video, onto the surface.
6. **Cursors.** Each user appears as a floating cursor with a name banner, in a color unique to them.
7. **Cursor visibility follows the view.** If your cursor is inside the area another user is looking at, they see
   it, and the same holds the other way round.
8. **Each user has their own view.** Only what is in a user's view is displayed. They can zoom in and out and pan
   across the whole surface.
9. **Infinite by default, bounded by choice.** A surface has no edges unless it is created with limits.

The goals in `vision.md` still hold: one core for native and WebAssembly, library first, good ink.

## The shape of the system

```
  shell (native or web)                       shell (native or web)
  window, input, clipboard,                   ...
  storage, network, media decode
        |  platform API
  +-----+--------------------------------+
  | everink-core                         |
  |  surface model: objects, order       |
  |  camera, spatial index, hit testing  |
  |  tools, selection, undo              |
  +----------+---------------+-----------+
  | -render  |    -sync      |
  | scene to | partial copy  |      one WebSocket per client
  | GPU      | of surface,   | <------------------------------+
  +----------+ presence      |                                |
             +---------------+                                v
                                         +------------------------------------+
                                         | everink-server (one room/surface)  |
                                         |  applies and stores changes        |
                                         |  spatial index of every object     |
                                         |  view subscriptions                |
                                         |  presence routing                  |
                                         |  blob store (images, video, fonts) |
                                         +------------------------------------+
```

Three ideas carry the design:

1. **Everything on the surface is an object** with a position, an order and a kind. Ink, shapes, text, images,
   video and placed library items differ only in their kind. Moving, grouping, ordering, selecting and undo are
   written once.
2. **The view drives everything.** The client tells the server what rectangle it is looking at. The server sends
   the objects that touch that rectangle and the cursors that are inside it, and nothing else. Opening a surface
   costs the same whether it holds a hundred objects or ten million.
3. **Two channels.** Changes to objects are durable and merge without conflicts. Presence (cursors, selections,
   ink still being drawn) is short-lived, sent often, never stored and dropped freely under load.

## The surface

A surface is one shared canvas. Its settings are fixed at creation or changed by its owner:

- **Bounds:** none (infinite, the default) or a rectangle. On a bounded surface the camera stops at the edge and the
  server refuses objects placed outside it. Nothing else in the system differs between the two.
- **Background** (color, optional grid), **zoom range**, the object kinds allowed, and size limits for pasted media.

**Coordinates.** Positions are 64-bit floating-point world units, where one unit is one pixel at 100% zoom.
Precision stays finer than a thousandth of a unit out to about a trillion units from the origin in each direction,
and the engine clamps there. That is not mathematically infinite; it is far beyond anything a person can pan to.
The renderer subtracts the camera position before anything reaches the GPU, so the graphics card only ever sees
small numbers and far-away content is as sharp as content at the origin.

**Zoom** is a bounded range (start with 2% to 6400%). An unlimited deep zoom is a different product and is not
planned.

## Objects

Every object has: an id, a kind, a transform (position, rotation, scale), an order key, an optional parent (for
groups), a style, and who created it.

| Kind | Holds | Notes |
|---|---|---|
| Stroke | pressure-aware points, brush | Fixed once the pen lifts; later edits move, restyle or erase it. |
| Shape | rectangle, ellipse, line, arrow, polygon or free path; fill and outline | |
| Text | rich text in a box (font, size, color, weight per run) | Edited by several people at once, character by character. |
| Image | reference to a stored file, its size, a tiny preview | The file lives in the blob store, not in the document. |
| Video | as image, plus a poster frame and duration | Later milestone. Each viewer plays it for themselves at first. |
| Group | child objects | Moves and scales as one. |

**Pre-drawn objects** come from **libraries**: named collections of templates, where a template is a group of
ordinary objects or an image. Placing one copies it onto the surface as normal, editable objects that remember
which template they came from. A library is itself stored as a small surface, so the same tools make them.

**Text looks the same everywhere** only if every client lays it out with the same fonts and the same layout code.
The core does the layout, and fonts are files shipped with Everink or stored in the blob store, never whatever
the operating system happens to have.

## How concurrent edits merge

The document is a map from object id to object. Within an object:

- Each property (position, style, parent) is a last-writer-wins value: two people moving the same object at once
  end with one of the two positions on every screen, never a mixture and never a conflict dialog.
- Order uses fractional keys, so placing one object between two others never renumbers anything.
- Text content is a sequence that merges concurrent typing.
- Deleting leaves a marker, so a delete and a concurrent edit agree.

These are standard CRDT building blocks, and both candidate libraries (Loro, yrs) provide them. Which library is
used stays an open owner decision; the spike in `vision.md` decides it. **Undo is per user:** it reverses your own
changes and leaves everyone else's alone. There are no locks. Other people's selections are shown in their color,
which is usually enough to stop two people grabbing the same thing.

**The object, not the tile, is the unit of sync.** This answers open question 3 in `vision.md`. An object keeps its
identity when it moves; only its entry in the spatial index changes. A long stroke that crosses many regions is
still one object and is sent to anyone whose view touches any part of it. Nothing is ever split at a boundary.

## Views and what gets sent

Each client holds a **partial copy** of the surface: the objects it has been sent.

1. The client sends its view rectangle (padded by half a screen on each side so panning does not show holes) and
   its zoom.
2. The server keeps a spatial index (an R-tree) of every object's bounding box. It sends each object that touches
   the rectangle and is not already on the client.
3. While the view stays put, the server forwards changes to those objects, and objects that move or are created
   inside the rectangle.
4. When the view moves, the client sends the new rectangle and the server sends the difference. The client keeps
   recently seen objects in a cache and drops the oldest when memory runs short.

**Zoomed far out,** a view can touch more objects than is useful to send. The server leaves out objects that would
be smaller than a pixel at the client's zoom. A later milestone adds **overview tiles**: small pre-rendered
pictures of busy regions, produced by a headless copy of the same core, so a zoomed-out view of a dense surface is
a few images, not a million strokes.

Because the server must know where every object is, it reads the document. It is not a blind relay, which changes
the "stateless fan-out" line in the `vision.md` sketch. End-to-end encryption of content is therefore not
possible in this design.

## Presence

Each client sends, about 30 times a second while something changes: cursor position, view rectangle, current
selection, the tool in use, and the points of any stroke still being drawn.

- The server gives each user a **color** that no one else present on that surface has, and passes on their
  **name**.
- **Routing follows requirement 7 exactly:** user A receives user B's cursor only while B's cursor is inside A's
  view rectangle. The server already has both facts.
- Clients smooth cursor motion between updates. If a connection is slow the server sends only the newest state.
- Ink appears on other screens while it is being drawn, through presence. When the pen lifts, the finished stroke
  is sent once as an object and replaces the preview.
- A list of who is on the surface, with "jump to this person", is cheap to add on the same channel.

## Rendering

Every frame: ask the local spatial index what touches the view, order it, and draw it with a GPU vector renderer
(Vello on wgpu is the candidate; it runs natively and on WebGPU). Three layers, back to front:

1. **Content:** the objects.
2. **Live ink:** the local user's stroke in progress, drawn with input prediction on the shortest path to the
   screen, because this is where latency is felt.
3. **Overlay:** selection handles, other users' selections and cursors with their name banners.

Images are decoded to textures at the resolution the current zoom needs; the blob store holds smaller versions so a
zoomed-out client never downloads a full-size photo. Video frames come from the platform's decoder through the
platform API and are drawn like a changing image.

## Media

Pasting an image creates the object at once with a local preview, then uploads the file in the background. Files
are named by a hash of their content, so the same picture pasted twice is stored once. Other users see a blurred
placeholder until their download finishes. Uploads go over HTTP, not the WebSocket, so a large file never delays
ink or cursors.

## The platform API

The core does no input or output itself. A shell gives it:

| Capability | Native | Web |
|---|---|---|
| Drawing surface | window | canvas element |
| Input: pointer with pressure, keyboard, wheel and gestures, text composition | OS events | DOM events |
| Clipboard and drag-and-drop | OS | browser |
| Local storage (cache, offline changes) | files | IndexedDB or OPFS |
| Network | WebSocket | WebSocket |
| Media decoding (video) | OS or library decoder | browser decoder |

The same core therefore also runs headless in the server and in the overview-tile renderer.

## The server

One room per surface. A room authenticates connections, checks and applies changes, keeps the spatial index,
serves view subscriptions, routes presence, and saves a change log with periodic snapshots. Rooms are independent,
so many surfaces spread across machines by surface id. The blob store is separate (a directory or any S3-style
store).

**Identity and permissions** are supplied by whoever embeds Everink: the server accepts a signed token carrying a
user id, a display name and a role (owner, editor or viewer) for a surface. Everink ships no account system.

**Offline.** A client keeps working on the objects it has. Its changes are saved locally and sent on reconnect,
where they merge by the rules above.

**Untrusted input.** The server checks every change (size, kind, bounds, rate). Images and video are data from
strangers: decoded by memory-safe decoders, limited in size, never executed. Text is plain runs, never markup.

## Packages

The README plans four. This design adds two and sharpens the roles; the README changes when the design is
accepted.

| Package | Role |
|---|---|
| `everink-core` | Surface model, camera, spatial index, hit testing, tools, undo. No I/O. |
| `everink-render` (new) | Turns the visible objects into GPU drawing; text layout; image and video textures. |
| `everink-sync` | Merge rules, wire protocol, partial copy, presence. |
| `everink-server` (new) | The room server and blob store interface. |
| `everink-native` | Desktop shell. |
| `everink-web` | Browser shell (wasm + WebGPU). |

## Targets to test

Proposed, not measured:

- Pen to visible ink: under 20 ms of software latency.
- 60 frames per second with 10,000 objects in view.
- Opening a surface: first content in under one second, whatever the surface's total size.
- Cursor updates 30 times a second; a remote cursor lags by no more than 100 ms on a good connection.
- 100,000 strokes in a client's cache within a memory budget set by the spike.

## Order of work

Each step is usable on its own and is the unit the detailed designs will follow.

| Step | Result |
|---|---|
| 0. Spike | One stroke type, two clients, a server, native and web. Picks the CRDT library and measures the targets. |
| 1. Single-user canvas | Camera, pan and zoom, ink, shapes, select, move, undo, saved locally. |
| 2. Shared surface | Whole-surface sync, cursors with names and colors, live ink. |
| 3. View-scoped sync | Server spatial index, view subscriptions, cursor routing by view, client cache. |
| 4. Text, images, libraries | Text editing, paste and drop, blob store, templates. |
| 5. Bounded surfaces, roles, offline | Surface settings, owner/editor/viewer, reconnect merge. |
| 6. Video and overview tiles | Platform video, the headless tile renderer. |

## Decisions this design needs from the owner

1. **Zoom:** a bounded range (proposed), or zoom without limit?
2. **Libraries:** a built-in set, libraries users make themselves, or both?
3. **Video:** each viewer plays it for themselves (proposed first), or playback synchronized for everyone?
4. **Who users are:** signed in through the embedding app (proposed), or may anyone with a link join under a name
   they type?
5. **The server reads content.** View-scoped sync requires it. Acceptable?
6. **Toolbars and menus:** built once in Rust and shared, or built natively by each shell?
7. **History:** is the current state enough, or should a surface be rewindable to an earlier time?
8. Still open from `vision.md`: the CRDT library and which first app proves the library.
