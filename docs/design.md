# High-level design

Status: proposal, 2026-11-11, with the owner's seven design decisions of that day built in (listed under
[Owner decisions](#owner-decisions)). It turns `vision.md` and the owner's product description into a system shape
that the detailed design in [`design/`](design/README.md) breaks down part by part. Where the detailed sections
found this document wrong, its text has been changed to match; [`design/README.md`](design/README.md) lists what
changed. Numbers in it are starting values to test, not measurements.

## What the product does

The owner's description (2026-11-11), restated as requirements. Where a requirement was sharpened by one of the
seven decisions, the decision is named.

1. **One surface, many users.** Several people use the same surface at the same time.
2. **Draw anything.** Freehand ink and any other graphics (shapes, lines, paths).
3. **Place pre-drawn objects** from a library, and move them around.
4. **Text** that can be placed and moved like any other object.
5. **Paste images and video** onto the surface.
6. **Cursors.** Each user appears as a floating cursor with a name banner, in a color unique to them.
7. **Cursor visibility follows the view.** If your cursor is inside the area another user is looking at, they see
   it, and the same rule applies the other way round.
8. **Each user has their own view.** Only what is in a user's view is displayed. They can pan across the whole
   surface and zoom in and out. Decision 1 makes the zoom unlimited.
9. **Infinite by default, bounded by choice.** A surface has no edges, and by decision 1 no zoom limit, unless it is
   created with limits.

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
   no limit, in both directions, without losing precision.
3. **The view drives everything.** The client tells the server what it is looking at. The server sends the objects
   that touch that view and the cursors inside it, and nothing else. For the client, opening a surface costs the
   same whether it holds a hundred objects or a million.
4. **Two channels.** Changes to objects are durable and merge without conflicts. Presence (cursors, selections,
   ink still being drawn) is short-lived, sent often, never stored and dropped freely under load.

## Positions: no edges and no zoom limit

One ordinary number per axis cannot do this. A 64-bit floating-point number carries about sixteen digits, so after
enough zooming two neighboring points become the same number. Everink therefore does not use one coordinate
system.

- The surface is a stack of **levels**, numbered by an integer. Level 0 is where a new surface starts. Each level
  is 1024 times finer than the one before it: level 1 is what you reach by zooming in 1024 times, level -1 by
  zooming out 1024 times. Levels continue without end in both directions.
- Each level is a grid of square **cells**, 1024 units on a side. One cell is exactly one unit of the level above,
  so every cell has one parent cell and 1024 by 1024 children. A cell is named by two whole numbers that may have
  as many digits as needed.
- A **position** is a level, a cell and an **offset** inside the cell. The offset is a 64-bit whole number of
  very small fixed steps, not a floating-point number, so every position has exactly one encoding and any two can
  be compared exactly.

An object is anchored at a position. Its own points are distances from that anchor in units of its level, so an
object can be larger than a cell.

**Which level.** A view is on level L when one unit of L covers about 1 to 1024 screen pixels; that number is
the zoom within the level. Zooming past the top of that range moves the view to level L+1, where one unit is about
one pixel again; the ranges overlap a little so the level does not flip back and forth. A level is only a unit of
measure: nothing is drawn or sent because of the level it is on. An
object is created on the level of the view it was drawn in, so on its own level it is normally between a thousandth
of a unit and about two thousand units across: one pixel to one screen about two thousand pixels wide, at either
end of the zoom range. An object that is scaled, pasted or placed far outside that band moves to the level that
suits its new size and keeps its id.

**Arithmetic.** Two positions are compared (how far is this object from the camera, does it cross the view) with
whole-number arithmetic of whatever length is needed, so that comparison is exact however many levels apart the
two are. Only the answer, a distance from the camera, becomes an ordinary 64-bit number for drawing, and near the
camera that number is small. The exact step runs when an object comes into view or the camera has moved far from
the point the cached numbers were measured from, not for every point on every frame. A curve that belongs to a
level far above the view is first cut down to the piece near the view, by a method that leaves what is drawn
inside the view unchanged.

**The same outline on every machine.** Exact comparison is only useful if every client starts from the same
numbers. An object's drawable outline (the edge of a stroke, the curve of an ellipse, a rotated shape) is worked
out once, on the object's own level, using only basic arithmetic that gives the same answer on every processor and
in WebAssembly. Rotation and scale are stored as the numbers of the transform, not as an angle, and no platform
math function is used. That outline is what the exact step cuts. Numbers that are not finite are refused. What is
promised is the same outline and the same bounds on every machine; graphics cards do not promise the same pixels.

The cost is bytes: a position takes about twenty more bits for each level deeper than level 0. So that one
hostile change cannot exhaust memory, every surface has a cap on how deep and how far out a position may be. The
default is far beyond anything a person can reach by hand, and the server refuses changes past it.

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

**Order** is one sequence across all levels. A large backdrop drawn from far out can sit behind or in front of
small things drawn deep inside it, as its order key says.

**Layers,** which `vision.md` lists in the document model, are not a separate concept at first: order and groups
cover what they do. A layer can be added later as a named top-level group that can be locked or hidden, without
changing the model.

**Libraries of pre-drawn objects.** A library is a named collection of templates; a template is a group of ordinary
objects or an image. Everink ships a small built-in set (basic shapes, arrows, notes), and users save their own
drawings as templates in libraries they can share. Placing a template copies it onto the surface as normal,
editable objects that remember which template they came from. A library is stored as a small surface, so the same
tools make and edit them.

**Text looks the same everywhere** only if every client lays it out with the same fonts and the same layout code.
The core does the layout, and fonts are files shipped with Everink or stored in the blob store, never whatever the
operating system happens to have. Shipped fonts carry their own licenses, which are checked when they are added.
A font a user uploads is untrusted data like any other file and is read only by a memory-safe parser.

## How concurrent edits merge

The document is a map from object id to object. Within an object:

- Each property (placement, style, parent, playback state) is a last-writer-wins value: two people moving the same
  object at once end with one of the two positions on every screen, never a mixture and never a conflict dialog.
  The server decides which write is last, by the order in which changes reach it. An object's level, position,
  rotation and scale are one property, replaced whole.
- Order uses fractional keys, so placing one object between two others never renumbers anything.
- Text content is a sequence that merges concurrent typing.
- Deleting leaves a marker, so a delete and a concurrent edit agree.

The detailed design proposes that Everink holds this map itself, ordered by the server, and uses a CRDT library
only for the text inside text objects. The reason is that each client holds part of a surface, and none of the
candidate libraries (Loro, yrs, Automerge) documents a way to sync part of a document. That finding is for the
spike to confirm, and the choice stays the owner's ([`design/02-document-model.md`](design/02-document-model.md)).
The server may refuse a change outright: one that would put a group inside itself, or that breaks a role or a
limit. **Undo is per user:** it reverses your own changes and leaves everyone else's alone. There are no locks.
Other people's selections are shown in their color, which is usually enough to stop two people grabbing the same
thing.

**The object, not the cell, is the unit of sync.** This answers open question 3 in `vision.md`. An object keeps its
identity when it moves; only its entry in the spatial index changes. A long stroke that crosses many cells is still
one object and is sent to anyone whose view touches any part of it. Nothing is ever split at a boundary.

## Views and what gets sent

A **view** is a level, a rectangle on that level and the zoom within the level. Each client holds a **partial
copy** of the surface: the objects it has been sent.

1. The client sends its exact view. The server adds half a screen on each side, so panning does not show holes.
2. The server keeps a spatial index of every object, filed by where it is and how large it is. From the index
   it sends **every object, on any level, that crosses the view and would cover at least a pixel on screen**.
   Size decides, not level: a backdrop drawn from far out and a note drawn deep inside it are found by the same
   test. On a view so dense that this would be too much to send, the server raises the size threshold and tells
   the client it has done so.
3. While the view stays put, the server forwards changes to those objects and anything that moves or is created
   inside the view.
4. When the view moves or changes level, the client sends the new view and the server sends the difference. The
   client keeps recently seen objects in a cache and drops the oldest when memory runs short.

**Content too small to see** is not sent. The server tells the client where such content lies, so the client can
mark "there is more here if you zoom in". A later step adds **overview pictures**: small pre-rendered
images of everything beneath a cell, produced by a headless copy of the same core, so a dense region seen from far
above is a few images, not a million strokes.

The server must know where every object is, so it reads the document (owner decision 5). It is not a blind relay,
which changes the "stateless fan-out" line in the `vision.md` sketch. Content is encrypted in transit and may be
encrypted on disk; it is not end-to-end encrypted.

## Presence

Each client sends, about 30 times a second while something changes: cursor position, zoom scale, current
selection, the tool in use, and the points of any stroke still being drawn. Its view is sent separately, to the
server only.

- The server gives each user a **color** that no one else present on that surface has, and passes on their
  **name**. The palette is finite (56 combinations of color and cursor shape in the detailed design); past its
  size they repeat and the name banner tells people apart.
- **Routing follows requirement 7 exactly, in each direction separately:** user A receives user B's cursor only
  while B's cursor position is inside A's view, and B receives A's only while A's cursor is inside B's view. The
  test is the exact position comparison, so zoom depth does not matter. If B is working a thousand times deeper at
  a spot inside A's view, A sees B's cursor there at normal size, with a note of how much deeper B is; B sees A's
  cursor only if it happens to fall inside B's much smaller view. A person zoomed far out moves their pointer in
  steps larger than a deep view, so their cursor seldom lands in one.
- Clients smooth cursor motion between updates. If a connection is slow the server sends only the newest state.
- Ink appears on other screens while it is being drawn, through presence. When the pen lifts, the finished stroke
  is sent once as an object and replaces the preview.
- The list of who is on the surface, with "jump to this person", runs on the same channel.

## Rendering

The content is drawn into a cache of square tiles with a GPU vector renderer, and each frame is composed from
those tiles. Panning moves tiles, zooming scales them while sharper ones are drawn, and a still scene costs almost
nothing. The renderer is the Vello family: on WebGPU where the browser has it, on WebGL2 where it does not, and on
the processor as a last resort, behind Everink's own interface. Positions are turned into distances from the camera
by the exact arithmetic above before anything reaches the GPU, so the graphics card only ever sees small numbers,
at any depth. Layers, back to front:

1. **Content:** the visible objects from every level, in their one order.
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
| Network: one WebSocket for changes and presence, HTTP for files | sockets | browser |
| Time and randomness | OS | browser |
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
(start with 30 days). A member with the editor role can save a **named version**, which is kept until deleted;
guests cannot manage versions. A version can be opened read-only, copied from, or restored. Restoring is itself a
change, so it can be reversed by restoring the snapshot from just before. Who may restore is a decision for the
owner; until it is made, only a surface's owner may. This is recovery from accidents, not a recording: the surface
cannot be replayed stroke by stroke.

**Offline.** A client keeps working on the objects it has. Its changes are saved locally and sent on reconnect,
where they merge by the rules above.

## Security

A first assessment, at the depth of this document. The detailed design of the server and of identity (step 2)
carries the full review; its open points are listed at the end.

| Who | Trusted to | Not trusted to |
|---|---|---|
| Server operator | read and store every surface (decision 5) | — |
| Embedding app | say who a user is and what role they have on its own surfaces, by signing tokens | issue tokens for another app's surfaces |
| Owner, editor | change the surface within their role | exceed the role, flood the room |
| Viewer | read | change anything |
| Guest (share link) | act within the link's role | be who their typed name says they are |
| Anyone on the network | nothing | read or alter traffic |

What the design does about the ways these can go wrong:

- **Every change is checked by the server** against the sender's role, the surface's limits, and size and rate
  caps, more strictly for guests. A client is never believed about who it is or what it may do.
- **A guest's name is a label, not an identity.** Guests are marked as guests on their banner.
- **Share links** are long random values, grant one role, and can be turned off; turning one off disconnects the
  guests who came through it. A link is a secret inside an address, and addresses leak through browser history
  and logs, so the secret sits in the part of the address a browser does not send onward and is exchanged for a
  session at once.
- **Tokens** expire and are bound to one tenant (one embedding app) and one surface. The server trusts one signing
  key per tenant, so one app's tokens are worthless on another's surfaces. How keys are rotated and tokens revoked
  is for the detailed design.
- **The blob store believes no one about a file's name.** The server computes the hash of what was uploaded
  itself. A file is read only through a surface that refers to it and that the reader may see, by a signed
  address tied to the reader's session, so ending the session ends the access. Knowing a file's hash grants
  nothing. Identical
  files are shared only within one tenant, so no one can learn from an upload whether a file exists elsewhere.
- **Pasted media is data from strangers:** images are decoded by memory-safe decoders in a separate, limited
  process (video is decoded by the browser or operating system, which is not memory-safe code), limited in size and
  dimensions, never
  executed. Text is plain runs, never markup. Files are served from the blob store as downloads, not as pages.
- **Presence gives away little by default:** a client receives only cursors inside its own view, and views are
  never sent to other clients. The list of people and "jump to this person" are the exception: they reveal, on
  request, where someone is working, and repeated jumps amount to following them. That is accepted for people
  sharing a surface; whether a person can opt out is for the detailed design.
- **Local copies stay on the machine.** The cache and offline changes are on disk or in browser storage. Signing
  out or leaving a guest session clears them.

Left open and accepted: the operator can read everything; anyone on a surface can find out where anyone else on it
is working; and a person with an editor link can deface a surface until the link is turned off. The remedies for
the last are an owner's "delete everything by this guest or link", freezing guests, and versions.

## When things fail

| Failure | What happens |
|---|---|
| A client loses its connection | It keeps working on what it has; changes queue locally and merge on reconnect. Its cursor disappears for others after a few seconds. |
| A room process crashes | Clients reconnect and resend unacknowledged changes; the room rebuilds from the last snapshot and the change log. Changes are acknowledged only once logged. |
| The spatial index disagrees with the document | The index is derived data: it is rebuilt from the document, on demand and on every room start. |
| The blob store is down | Drawing, text and cursors carry on. New media shows as a placeholder marked as not yet uploaded, and uploads retry. |
| Video clocks drift | Each client re-measures its clock offset regularly and nudges playback; a large error is corrected by a seek. |
| Someone floods changes or presence | Per-connection rate caps; presence beyond the cap is dropped, changes beyond it are refused, and repeat offenders are disconnected. |
| A restore happens while others are editing | Their in-flight changes apply on top of the restored state by the normal merge rules. The detailed design decides whether to warn them first. |

## What proves it

Each rule the design depends on has a test that fails if the rule is broken, written in the step that introduces it.

| Rule | Proved by | Step |
|---|---|---|
| A point drawn at any depth is found at exactly the same place later, on every client | round-trip tests over random positions and levels; two clients compare | 0 |
| A shape from many levels up has the same outline and bounds on every client | byte-for-byte comparison on x86-64, aarch64 and WebAssembly, and tests against a second big-number library | 1 |
| Concurrent changes converge to the same state on every client | randomized tests applying the same changes in different orders | 0 |
| Delete and a concurrent edit agree | the same randomized tests, with deletes | 0 |
| Undo reverses only your own changes, also after someone else moved the object | scripted two-user scenarios | 1, 2 |
| A client is sent every object that crosses its view and is at least a pixel, and nothing from outside the wider region it has not yet left | compare the server's answer with a brute-force scan | 3 |
| A cursor is drawn exactly when it is inside the receiver's view (it is delivered a little early, within a small margin) | scripted scenarios across levels | 3 |
| A role cannot be exceeded | a negative test per role and change type | 2, 5 |

## Packages

The README plans four. This design adds four and sharpens the roles; the README table changes when the packages
are created.

| Package | Role |
|---|---|
| `everink-core` | Surface model, positions, camera, spatial index, hit testing, tools, undo. No I/O. |
| `everink-render` (new) | Turns the visible objects into GPU drawing; text layout; image and video textures. |
| `everink-ui` (new) | Shared toolbars and menus. |
| `everink-sync` | Merge rules, wire protocol, partial copy, presence. |
| `everink-runtime` (new) | Runs what the core asks for (network, storage, clipboard, media) through the platform API. |
| `everink-server` (new) | The room server, versions and blob store interface. |
| `everink-native` | Desktop shell. |
| `everink-web` | Browser shell (wasm + WebGPU). |

## Targets to test

Proposed, not measured:

- Pen to visible ink: under 4 ms from pointer event to GPU submit, and a measured pen-to-ink figure on named
  devices (the first version of this target, 20 ms, did not say what was measured).
- 60 frames per second with 10,000 objects in view, at level 0 and at level 100 alike.
- Zooming continuously through a level boundary shows no jump.
- The exact position arithmetic stays within a budget of 2 ms per frame while panning at level 100.
- Opening a surface: first content in under one second, whatever the surface's total size.
- Cursor updates 30 times a second; a remote cursor lags by under 200 ms on a good connection, of which
  smoothing is under 100 ms.
- Two viewers of a playing video are within 100 ms of each other 95 times in 100, on connections with round
  trips under 60 ms.
- 100,000 strokes in a client's cache within a memory budget set by the spike.

## Order of work

Each step is usable on its own and is the unit the detailed designs will follow.

| Step | Result |
|---|---|
| 0. Spike | One stroke type, two clients, a server, native and web, with level-and-cell positions. Picks the CRDT library and measures the targets. |
| 1. Single-user canvas | Camera with unlimited pan and zoom, ink, shapes, select, move, undo, saved locally, first toolbars, the text-entry bridge. |
| 2. Shared surface | Whole-surface sync, cursors with names and colors, live ink, tokens and share links. |
| 3. View-scoped sync | Server spatial index across levels, view subscriptions, cursor routing by view, client cache, jump to person. |
| 4. Text, images, libraries | Text editing, paste and drop, blob store and media worker, built-in and user libraries. |
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

## The detailed design

The points this document left open are settled, or put to the owner, in [`design/`](design/README.md): eight
sections, the decisions that cross them, the questions the spike must answer and the decisions that need the
owner.
