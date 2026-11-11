# High-level design

Status: proposal, 2026-11-11, with the owner's seven design decisions of that day built in (listed under
[Owner decisions](#owner-decisions)). It turns `vision.md` and the owner's product description into a system shape
that later documents break down part by part. Numbers in it are starting values to test, not measurements.

## What the product does

The owner's description (2026-11-11), restated as requirements:

1. **One surface, many users.** Several people use the same surface at the same time.
2. **Draw anything.** Freehand ink and any other graphics (shapes, lines, paths).
3. **Place pre-drawn objects** from a library, and move them around.
4. **Text** that can be placed and moved like any other object.
5. **Paste images and video** onto the surface.
6. **Cursors.** Each user appears as a floating cursor with a name banner, in a color unique to them.
7. **Cursor visibility follows the view.** If your cursor is inside the area another user is looking at, they see
   it, and the same holds the other way round.
8. **Each user has their own view.** Only what is in a user's view is displayed. They can pan across the whole
   surface and zoom in and out without limit.
9. **Infinite by default, bounded by choice.** A surface has no edges and no zoom limit unless it is created with
   them.

The goals in `vision.md` still hold: one core for native and WebAssembly, library first, good ink.

## The shape of the system

```
  shell (native or web)                       shell (native or web)
  window, input, clipboard,                   ...
  storage, network, media decode
        |  platform API
  +-----+--------------------------------+
  | everink-ui: toolbars, menus          |
  +--------------------------------------+
  | everink-core                         |
  |  surface model: objects, order       |
  |  positions, camera, spatial index    |
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
                                         |  presence routing, shared clock    |
                                         |  versions                          |
                                         |  blob store (images, video, fonts) |
                                         +------------------------------------+
```

Four ideas carry the design:

1. **Everything on the surface is an object** with a position, an order and a kind. Ink, shapes, text, images,
   video and placed library items differ only in their kind. Moving, grouping, ordering, selecting and undo are
   written once.
2. **A position is a level, a cell and an offset.** That one structure gives a surface with no edges and zoom with
   no limit, in both directions, with exact arithmetic.
3. **The view drives everything.** The client tells the server what it is looking at. The server sends the objects
   that touch that view and the cursors inside it, and nothing else. Opening a surface costs the same whether it
   holds a hundred objects or ten million.
4. **Two channels.** Changes to objects are durable and merge without conflicts. Presence (cursors, selections,
   ink still being drawn) is short-lived, sent often, never stored and dropped freely under load.

## Positions: no edges and no zoom limit

Ordinary numbers cannot do this. A 64-bit number carries about sixteen digits, so after enough zooming two
neighboring points become the same number. Everink therefore does not use one coordinate system.

- The surface is a stack of **levels**, numbered by an integer. Level 0 is where a new surface starts. Each level
  is 1024 times finer than the one before it: level 1 is what you reach by zooming in 1024 times, level -1 by
  zooming out 1024 times. Levels continue without end in both directions.
- Each level is a grid of square **cells**, 1024 units on a side. One cell is exactly one unit of the level above,
  so every cell has one parent cell and 1024 by 1024 children. A cell is named by two whole numbers that may have
  as many digits as needed.
- A **position** is a level, a cell and an **offset** inside the cell (two ordinary 64-bit numbers).

Every calculation the engine does (where is this object relative to the camera, do these two things overlap) is
between things on the same or neighboring levels, where the numbers are small and exact. Nothing ever needs a
number larger than a cell. The cost is that a position deep in the stack takes more bytes: about twenty bits per
level away from level 0. The only limit is memory.

An object lives on the level that suits its size. A sketch drawn while zoomed in by a factor of a million lives two
levels down, and stays exactly as precise as one drawn at level 0.

**Bounded surfaces** are the same engine with limits set: a rectangle the content must stay inside, a range of
levels the camera may visit, or both. The camera stops at a limit and the server refuses changes beyond it.

**Finding your way.** With no edges and no zoom limit it is easy to get lost, so navigation is part of the design:
a home position per surface, "jump to this person", and saved places anyone can return to.

## Objects

Every object has: an id, a kind, a position, a rotation and scale, an order key, an optional parent (for groups), a
style, and who created it.

| Kind | Holds | Notes |
|---|---|---|
| Stroke | pressure-aware points, brush | Fixed once the pen lifts; later edits move, restyle or erase it. |
| Shape | rectangle, ellipse, line, arrow, polygon or free path; fill and outline | |
| Text | rich text in a box (font, size, color, weight per run) | Edited by several people at once, character by character. |
| Image | reference to a stored file, its size, a tiny preview | The file lives in the blob store, not in the document. |
| Video | as image, plus a poster frame, duration and shared playback state | See [Video](#video). |
| Group | child objects | Moves and scales as one. |

**Libraries of pre-drawn objects.** A library is a named collection of templates; a template is a group of ordinary
objects or an image. Everink ships a small built-in set (basic shapes, arrows, notes), and users save their own
drawings as templates in libraries they can share. Placing a template copies it onto the surface as normal,
editable objects that remember which template they came from. A library is stored as a small surface, so the same
tools make and edit them.

**Text looks the same everywhere** only if every client lays it out with the same fonts and the same layout code.
The core does the layout, and fonts are files shipped with Everink or stored in the blob store, never whatever the
operating system happens to have.

## How concurrent edits merge

The document is a map from object id to object. Within an object:

- Each property (position, style, parent, playback state) is a last-writer-wins value: two people moving the same
  object at once end with one of the two positions on every screen, never a mixture and never a conflict dialog.
- Order uses fractional keys, so placing one object between two others never renumbers anything.
- Text content is a sequence that merges concurrent typing.
- Deleting leaves a marker, so a delete and a concurrent edit agree.

These are standard CRDT building blocks, and both candidate libraries (Loro, yrs) provide them. Which library is
used stays an open owner decision; the spike in `vision.md` decides it. **Undo is per user:** it reverses your own
changes and leaves everyone else's alone. There are no locks. Other people's selections are shown in their color,
which is usually enough to stop two people grabbing the same thing.

**The object, not the cell, is the unit of sync.** This answers open question 3 in `vision.md`. An object keeps its
identity when it moves; only its entry in the spatial index changes. A long stroke that crosses many cells is still
one object and is sent to anyone whose view touches any part of it. Nothing is ever split at a boundary.

## Views and what gets sent

A **view** is a level, a rectangle on that level and a zoom within the level. Each client holds a **partial copy**
of the surface: the objects it has been sent.

1. The client sends its view, padded by half a screen on each side so panning does not show holes.
2. The server keeps a spatial index of every object: for each level, the cells that hold objects and the objects in
   each, and for each cell whether anything lies in the levels beneath it. From the index it sends:
   - objects on the view's own level that touch the rectangle;
   - objects from the two levels below that are large enough to see;
   - objects from the levels above that cross the view. Seen from below these are enormous, so there are few: the
     view is usually either inside one of them or clear of all of them.
3. While the view stays put, the server forwards changes to those objects and anything that moves or is created
   inside the view.
4. When the view moves or changes level, the client sends the new view and the server sends the difference. The
   client keeps recently seen objects in a cache and drops the oldest when memory runs short.

**Content too small to see.** Anything more than two levels below the view is smaller than a pixel. It is not sent.
Because the index knows which cells have content beneath them, the client can mark "there is more here if you zoom
in". A later step adds **overview pictures**: small pre-rendered images of everything beneath a cell, produced by a
headless copy of the same core, so a dense region seen from far above is a few images, not a million strokes.

The server must know where every object is, so it reads the document (owner decision 5). It is not a blind relay,
which changes the "stateless fan-out" line in the `vision.md` sketch. Content is encrypted in transit and may be
encrypted on disk; it is not end-to-end encrypted.

## Presence

Each client sends, about 30 times a second while something changes: cursor position, view, current selection, the
tool in use, and the points of any stroke still being drawn.

- The server gives each user a **color** that no one else present on that surface has, and passes on their
  **name**.
- **Routing follows requirement 7 exactly:** user A receives user B's cursor only while B's cursor position is
  inside A's view. Zoom depth does not matter. If B is working a thousand times deeper at a spot inside A's view, A
  sees B's cursor at that spot, drawn at normal size with a note of how much deeper B is.
- Clients smooth cursor motion between updates. If a connection is slow the server sends only the newest state.
- Ink appears on other screens while it is being drawn, through presence. When the pen lifts, the finished stroke
  is sent once as an object and replaces the preview.
- The list of who is on the surface, with "jump to this person", runs on the same channel.

## Rendering

Every frame: ask the local index what touches the view, order it, and draw it with a GPU vector renderer (Vello on
wgpu is the candidate; it runs natively and on WebGPU). Positions are converted to distances from the camera before
anything reaches the GPU, so the graphics card only ever sees small numbers, at any depth. Layers, back to front:

1. **Content:** the objects, from the levels above the view down to the smallest visible.
2. **Live ink:** the local user's stroke in progress, drawn with input prediction on the shortest path to the
   screen, because this is where latency is felt.
3. **Overlay:** selection handles, other users' selections and cursors with their name banners.
4. **Controls:** toolbars and menus (below).

Images are decoded to textures at the resolution the current zoom needs; the blob store holds smaller versions so a
zoomed-out client never downloads a full-size photo. Zooming far into an image shows its pixels; vector content
stays sharp at any depth.

## Toolbars and menus

The controls around the canvas are built once in Rust, in `everink-ui`, and drawn by Everink on the same GPU
surface, so they are identical on desktop and web (owner decision 6). An app that embeds Everink can hide them and
drive the canvas through commands (set tool, set color, undo) with its own controls.

Two things this choice makes Everink responsible for, which native controls would have provided: **accessibility**
(a screen reader cannot see controls drawn on a canvas, so the shell must expose a description of them to the
platform) and **text entry** (composition for languages that need it, on-screen keyboards). Both are named in the
platform API and are a known cost of this decision.

## Media

Pasting an image or video creates the object at once with a local preview, then uploads the file in the
background. Files are named by a hash of their content, so the same picture pasted twice is stored once. Other
users see a blurred placeholder until their download finishes. Transfers go over HTTP, not the WebSocket, so a
large file never delays ink or cursors.

### Video

Playback is **synchronized for everyone** (owner decision 3). A video object carries shared playback state:
playing or paused, and the media time at a moment on the server's clock. Pressing play, pause or seek changes that
state for all viewers, by the same last-writer-wins rule as any other property.

- Each client measures its offset from the server's clock and works out where the video should be now.
- A client that falls behind (buffering, a slow machine) catches up by skipping forward; it never pauses the others.
- Sound plays only while the video is in your view, with a volume and mute of your own.
- Browsers refuse to start sound before the person has interacted with the page, so a web client may show a video
  playing silently with a prompt to unmute.

Frames come from the platform's decoder through the platform API and are drawn like a changing image.

## The platform API

The core does no input or output itself. A shell gives it:

| Capability | Native | Web |
|---|---|---|
| Drawing surface | window | canvas element |
| Input: pointer with pressure, keyboard, wheel and gestures, text composition | OS events | DOM events |
| Clipboard and drag-and-drop | OS | browser |
| Accessibility: publish a description of the controls and selection | OS accessibility API | a hidden DOM mirror |
| Local storage (cache, offline changes) | files | IndexedDB or OPFS |
| Network | WebSocket | WebSocket |
| Media decoding and sound (video) | OS or library decoder | browser decoder |

The same core therefore also runs headless in the server and in the overview-picture renderer.

## The server

One room per surface. A room authenticates connections, checks and applies changes, keeps the spatial index, serves
view subscriptions, routes presence, provides the shared clock, and saves a change log with snapshots. Rooms are
independent, so many surfaces spread across machines by surface id. The blob store is separate (a directory or any
S3-style store).

**Who users are** (owner decision 4). Two ways onto a surface:

- **A signed token** from the app that embeds Everink, carrying a user id, a display name and a role (owner, editor
  or viewer). Everink ships no account system of its own.
- **A share link**, if the surface's owner turns one on, with the role it grants. Anyone with the link joins as a
  guest under a name they type. Guests get a color and a cursor like anyone else; the owner can turn the link off.

**Versions** (owner decision 7). The server takes snapshots as a surface changes and keeps them for a set period
(start with 30 days). Anyone with the editor role can save a **named version**, which is kept until deleted. A
version can be opened read-only, copied from, or restored; restoring is itself a change, so it can be reversed by
restoring the snapshot from just before. This is recovery from accidents, not a recording: the surface cannot be
replayed stroke by stroke.

**Offline.** A client keeps working on the objects it has. Its changes are saved locally and sent on reconnect,
where they merge by the rules above.

**Untrusted input.** The server checks every change (size, kind, bounds, rate), more strictly for guests. Images
and video are data from strangers: decoded by memory-safe decoders, limited in size, never executed. Text is plain
runs, never markup.

## Packages

The README plans four. This design adds three and sharpens the roles; the README changes when the design is
accepted.

| Package | Role |
|---|---|
| `everink-core` | Surface model, positions, camera, spatial index, hit testing, tools, undo. No I/O. |
| `everink-render` (new) | Turns the visible objects into GPU drawing; text layout; image and video textures. |
| `everink-ui` (new) | Shared toolbars and menus. |
| `everink-sync` | Merge rules, wire protocol, partial copy, presence. |
| `everink-server` (new) | The room server, versions and blob store interface. |
| `everink-native` | Desktop shell. |
| `everink-web` | Browser shell (wasm + WebGPU). |

## Targets to test

Proposed, not measured:

- Pen to visible ink: under 20 ms of software latency.
- 60 frames per second with 10,000 objects in view, at level 0 and at level 100 alike.
- Zooming continuously through a level boundary shows no jump, and a point drawn at any depth is found at exactly
  the same place after zooming away and back.
- Opening a surface: first content in under one second, whatever the surface's total size.
- Cursor updates 30 times a second; a remote cursor lags by no more than 100 ms on a good connection.
- Two viewers of a playing video are within 100 ms of each other.
- 100,000 strokes in a client's cache within a memory budget set by the spike.

## Order of work

Each step is usable on its own and is the unit the detailed designs will follow.

| Step | Result |
|---|---|
| 0. Spike | One stroke type, two clients, a server, native and web, with level-and-cell positions. Picks the CRDT library and measures the targets. |
| 1. Single-user canvas | Camera with unlimited pan and zoom, ink, shapes, select, move, undo, saved locally, first toolbars. |
| 2. Shared surface | Whole-surface sync, cursors with names and colors, live ink, tokens and share links. |
| 3. View-scoped sync | Server spatial index across levels, view subscriptions, cursor routing by view, client cache, jump to person. |
| 4. Text, images, libraries | Text editing, paste and drop, blob store, built-in and user libraries. |
| 5. Bounded surfaces, roles, versions, offline | Surface limits, owner/editor/viewer, named versions, reconnect merge. |
| 6. Video and overview pictures | Synchronized video, the headless overview renderer. |

## Owner decisions

Asked one at a time on 2026-11-11; the options as offered and the answers are quoted in
https://github.com/vchance/everink/pull/5.

| # | Question | Decision |
|---|---|---|
| 1 | Zoom | Unlimited deep zoom |
| 2 | Libraries of pre-drawn objects | Both: a built-in set and libraries users make |
| 3 | Video playback | Synchronized for everyone |
| 4 | Who users are | Both: signed tokens from an embedding app, and guest share links |
| 5 | The server reads content | Yes |
| 6 | Toolbars and menus | Shared, built once in Rust |
| 7 | History | Named versions |

Still open from `vision.md`: the CRDT library, and which first app proves the library.

## Points for the detailed designs

Not owner decisions yet; each detailed design settles its own and raises what the owner should choose.

- The level ratio (1024 here) and how an object's level is chosen and changed when it is scaled.
- How a view that straddles a level boundary is drawn and subscribed to.
- The library for the shared controls, and how far the accessibility mirror goes in the first version.
- How long automatic snapshots are kept, and what a restore does to people who are editing at that moment.
- What guests may do by default, and limits on what they can upload.
