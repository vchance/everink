# Detailed design

Status: proposal, 2026-11-11. Eight sections break down the high-level design in [`../design.md`](../design.md).
Each was researched and written separately, against prior art and the current state of the libraries it would use,
and then reconciled with the others. Nothing here is built or measured. Every number is a starting value, and each
section ends with the claims it could not verify.

## Sections

| # | Section | What it settles |
|---|---|---|
| 01 | [Positions, camera and deep zoom](01-positions.md) | The position type, the camera, exact arithmetic, cutting large shapes, re-levelling, the depth cap |
| 02 | [Document model, merge and undo](02-document-model.md) | Every object kind field by field, how edits merge, order, groups, delete, undo, partial erase |
| 03 | [View-scoped sync, protocol, server and storage](03-sync-and-server.md) | The server's index, what is sent for a view, the wire protocol, storage, versions, rooms |
| 04 | [Presence, cursors and live ink](04-presence.md) | Cursor routing, smoothing, colors, name banners, live ink, the people list, jump and follow |
| 05 | [Media: images, video, fonts and the blob store](05-media.md) | Accepted formats, the image pyramid, uploads, garbage collection, synchronized video, fonts |
| 06 | [Rendering, ink, text and level of detail](06-rendering-and-ink.md) | The renderer and its fallbacks, the tile cache, the ink pipeline, text layout, overview pictures |
| 07 | [Controls, platform API, shells and embedding](07-ui-and-platform.md) | The controls toolkit, the platform traits, the two shells, accessibility, text entry, the embedding API |
| 08 | [Identity, roles, limits and the threat model](08-identity-and-security.md) | Tokens, share links, the permission matrix, every numeric limit, the threat model |

Each section has the same shape: scope, the problem, what others do, options considered, the design, failure
modes, how it is tested, dependencies, what it assumes of other sections, questions for the owner, sources.

## How the sections fit

These are the decisions that cross section boundaries. The section named first holds the specification.

| Topic | Decision | Sections |
|---|---|---|
| Position | A level plus one fixed-point integer per axis, in steps of 2^-54 of a level unit. About 19 bytes at the origin and 2.5 more per level. | 01 |
| Depth cap | Levels within ±1024; an encoded position is at most about 3.1 KB. | 01, 08 |
| Placement | Level, anchor, rotation and scale are one value that is replaced whole. | 01, 02 |
| Groups | A child is placed relative to its parent, so moving a group is one write. A child is at most 8 levels from its group. | 02, 01, 03 |
| Merge | The object map is Everink's own and is ordered by the server: each field is last-writer-wins by the surface's sequence number. A CRDT library is used only for the text inside text objects. Proposed, for the owner to accept. | 02, 03 |
| What is sent | An object is sent when it crosses the padded view and covers at least one pixel, with a budget that raises the threshold on dense views. Size decides, not level. The client sends its exact view; the server adds the padding. | 03, 06 |
| Connection | One WebSocket carries changes and presence. Presence has its own queue, served first; bulk data is paced to what the link drains in about 20 ms, so a cursor waits about 40 ms at most behind it on links faster than about 1.6 Mbit/s. A second socket is the fallback if the spike shows this is not enough. | 03, 04, 07, 08 |
| Stroke data | Stored points: a 32-bit grid at 2^-16 of a unit plus pressure, fixed when the pen lifts. Derived outlines: a 64-bit grid at 2^-40, built with arithmetic that is identical on every machine. | 06, 01, 02 |
| Partial erase | A stroke carries a set of cuts, so two people erasing parts of one stroke both take effect. | 02, 06 |
| Renderer | `vello_gpu` on WebGPU, then on WebGL2, then `vello_cpu`, behind Everink's own interface. Frames come from a cache of 512-pixel tiles. | 06 |
| Controls | Everink's own small toolkit on its own renderer, with AccessKit for accessibility. | 07, 06 |
| Limits | One table holds every numeric limit. | 08 |
| Blob access | One grant per blob, tied to the session, checked against revoked sessions on every request. | 08, 05 |
| Clock | One offset from the server's clock, measured by ping, used by presence and video. | 03, 05, 04 |

## What changed from the high-level design

The research overturned or sharpened these points. [`../design.md`](../design.md) is amended to match.

- **Offsets are fixed-point, not floating-point.** Floating-point offsets cannot be compared exactly in bounded
  space and have several encodings of one value (01).
- **No CRDT library can hold the object map.** View-scoped sync needs each client to hold part of a surface, and
  none of Loro, yrs or Automerge documents a way to sync part of a document. Two sections reached this separately.
  It is unproven and the spike tests it (02, 03).
- **The server may refuse a change:** a move that would make a group contain itself, a role or limit violation, a
  move into a deleted group. "Everything always merges" is too strong (02).
- **Frames are not redrawn from the objects each time.** A tile cache makes a still scene nearly free and keeps
  10,000 objects at 60 frames per second (06).
- **"Vello" is several crates,** and the one meant for production is pre-1.0. The renderer is wrapped and only a
  safe subset of its features is used (06).
- **"Two levels below the view" is retired.** What is searched and drawn follows from size (03, 01).
- **"The same pixel on every client" becomes "the same outline and bounds on every client".** Graphics cards do
  not promise identical pixels (01, 06).
- **Targets restated:** remote cursors within 200 ms, not 100 ms, once motion is smoothed (04); video within
  100 ms at the 95th percentile on a good connection (05); pen latency defined as a measured figure on named
  devices (06).
- **The server keeps a whole surface in memory** in the first version, capped at 2 million objects. Opening a
  surface costs the same whatever its size for the client, not for the server (03).
- **Guests cannot manage versions.** "Anyone with the editor role" would have let a guest on an editor link
  restore or delete versions (08).
- **Versions are not the only remedy for defacement.** An owner can also delete everything by one guest or link,
  and freeze guests (08).
- **Knowing a file's hash is not permission to read it.** Access is granted per surface (08, 05).
- **The platform needs HTTP, time and randomness,** not only a WebSocket (07, 05).

## Packages

| Package | Role |
|---|---|
| `everink-core` | Surface model, positions, camera, local index, hit testing, tools, undo. No input or output. |
| `everink-render` | The renderer interface and back ends, the tile cache, ink outlines, text layout. |
| `everink-ui` | The shared controls. |
| `everink-sync` | Merge rules, wire protocol, partial copy, presence. |
| `everink-runtime` | Runs what the core asks for (network, storage, clipboard, media) through the platform traits. |
| `everink-server` | The room server, storage, versions, the blob store interface. The media worker is a separate process. |
| `everink-native` | Desktop shell. |
| `everink-web` | Browser shell. |

## Order of work

As in the high-level design, with two changes: the text-entry bridge is needed in step 1 (the color picker has a
text field), and the media worker in step 4.

### Step 0: the spike

One stroke type, two clients, a server, native and web. Its purpose is to replace the largest guesses in these
documents with measurements, before anything is built on them.

| Question | Section | What settles it |
|---|---|---|
| Can any of Loro, yrs or Automerge sync part of a document? | 02, 03 | A code check and a small trial of each |
| Which library edits text best in one small document per text object? | 02 | Size and memory per document; whether the server can inspect an update before applying it |
| Does Everink's own object map hold up? | 02, 03 | 100,000 strokes: memory per object, open time, randomized convergence tests |
| What does exact position arithmetic cost? | 01 | Re-base and cut timings at levels 0, 100 and 1,000 |
| Are outlines bit-identical everywhere? | 01, 06 | The same strokes on x86-64, aarch64 and wasm32, compared byte for byte |
| Does the renderer carry the load? | 06 | `vello_gpu` on WebGPU and on WebGL2: tile throughput, 10,000 objects, the size of the wasm build |
| What is pen latency really? | 06 | A 240 frames-per-second camera on named devices, with and without the low-latency canvas |
| Do cursors stall during a bulk load on one connection? | 04, 03 | Cursor delay while a client loads a large surface over a slow link |
| Is the purpose-built controls toolkit realistic? | 07 | One toolbar and one text field with composition and a screen reader, native and web |
| How fast is storage? | 03 | Commit latency and load time for one SQLite file per room |

## Decisions for the owner

Each section lists its own questions. These are the ones that shape the work, most consequential first.

1. **The merge model.** Accept Everink's own server-ordered object map, with a CRDT library only for text? This
   is the reserved CRDT decision in a new form: which library becomes a question about text alone (02, 03).
2. **The first app** (reserved). Sections 02, 03, 05, 06 and 07 each say what it changes. Section 01 notes that
   only an app that uses deep zoom justifies its cost.
3. **Browsers without WebGPU.** Supported through the WebGL2 path, with the CPU path as best effort (06, 07)?
4. **The controls toolkit.** Accept the risk of building one, with the spike as the check, or start on egui and
   accept two text stacks and no accessibility on the web (07)?
5. **Offline edits.** A long offline edit that reconnects overrides newer online edits to the same property,
   because the server orders by arrival. Acceptable (02)?
6. **Who may restore a version,** and whether other editors are warned first. Owner-only until decided (03, 08).
7. **Jump and follow.** Is everyone jumpable by default, may a person opt out, is the followed person told, and
   may an owner turn it off for guests (04, 08)?
8. **Guests.** Do they get cursors by default, what may they upload (images only is recommended), and how long
   does a share link last by default (30 days is recommended) (04, 05, 08)?
9. **The depth cap.** Are ±1024 levels and 3.1 KB positions acceptable, and is the cap set per surface or by
   whoever runs the server (01)?
10. **Erasing.** Which eraser is the default tool: whole objects (recommended) or parts of strokes (06, 02)?
11. **Media.** Animated images refused or turned into silent looping video; SVG import later; video outside
    H.264 and VP9 refused or converted by an operator-run tool; who may download originals, which can carry
    location data; custom fonts and their embedding flags (05).
12. **A stroke in progress when its author disconnects** is lost. Acceptable, or should the server keep it (04)?
13. **Scale and retention.** The 2-million-object cap; thinning automatic versions (all for a day, hourly for a
    week, daily to 30 days); how much may be lost if a server's disk dies; how long old protocol versions are
    supported (03).
14. **Languages and fonts.** Which scripts and which interface languages at first release; large fonts bundled or
    fetched (06, 07, 05).
15. **Accessibility commitment** for the built-in controls (07).

## What is not verified

- **Every performance figure is an estimate** from operation counts or stated assumptions. The spike replaces
  them.
- **Claims about other products** (Figma, Miro, tldraw, Excalidraw, Liveblocks) often rest on secondary sources.
  Each section marks which.
- **Library facts** (versions, licenses, behavior) were read on 2026-11-11 and are marked where a primary source
  could not be opened. Re-check a claim before relying on it.
- **Browser behavior** (WebGPU availability, low-latency canvas, storage eviction, autoplay) changes; each
  section dates what it read.
