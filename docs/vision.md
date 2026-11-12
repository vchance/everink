# Vision

## Problem

People want one shared drawing surface that never runs out of room, that several people can draw on at once,
and that works the same in a desktop app and a browser tab. Today you pick two of the three:

- Web whiteboards (tldraw, Excalidraw) are infinite and multi-user, but web-only.
- Drawpile is multi-user, native and in the browser, but it is a raster painting app with a bounded canvas,
  not a library.
- Native ink and vector tools (Graphite, google/ink) are not multi-user.

## Goals

1. **Infinite.** Pan and zoom without edges or precision loss far from the origin.
2. **Multi-user.** Concurrent editing that merges without conflicts, offline edits that merge on reconnect,
   and live presence (cursors, selections, strokes in progress).
3. **One core, two targets.** The same core library compiled natively and to wasm, with thin platform shells.
4. **Library first.** Embeddable in other apps, not only a standalone product.
5. **Good ink.** Low-latency, pressure-aware strokes with prediction.

## Non-goals (for now)

- A full design tool (components, prototyping, export to code).
- Raster painting with brush engines and layers of pixels.
- Hosting a public service. Everink ships a relay; operators run it.

## The gap Everink fills

None of the existing projects packages these together:

1. **A canvas document model built for CRDTs.** Strokes, shapes, groups, layers and z-order that merge
   cleanly, with per-user undo that reverses only your own changes.
2. **Viewport-scoped sync.** Load and subscribe to only the region a user is viewing, so an endless canvas
   stays cheap to open.
3. **Presence on a separate channel.** Cursors and in-progress strokes are ephemeral and must not grow the
   document.
4. **A shared core.** Native and wasm builds from one codebase, behind a small platform API (input, surface,
   storage, network).

## Architecture sketch

```
            +------------------- everink-core --------------------+
 input -->  | stroke modeling -> document model (CRDT) -> spatial  |
            | index (R-tree / tiles) -> visible set -> renderer    |
            +------------------------------------------------------+
                   ^                          |
                   | ops / snapshots          | draw calls (GPU)
            +------+------- everink-sync ----+------+
            | doc channel (CRDT updates, per tile)  |
            | presence channel (ephemeral)          |
            +---------------------------------------+
                   ^
                   | WebSocket
              relay server (stateless fan-out + snapshot store)
```

Coordinates: 64-bit world coordinates, or tile-local origins, so precision holds anywhere on the canvas.

[design.md](design.md) develops this sketch into a proposed design. Where the two differ, the design is the newer
thinking:

- The server reads the document and keeps an index of it; it is not a stateless relay. "Relay" in this document
  means that server.
- Sync is per object, not per tile.
- Positions are a level, a cell and an offset, so that zoom has no limit.
- Layers are not a separate part of the document model at first; order and groups cover them.

## Candidate stacks

| | Rust | C++ |
|---|---|---|
| Rendering | Vello on wgpu (native + WebGPU) | Skia (CanvasKit for wasm) |
| Sync | Loro or yrs | yrs via its C bindings, or Automerge C |
| Ink | port of perfect-freehand, or ink-stroke-modeler via FFI | google/ink, ink-stroke-modeler |
| Spatial index | `rstar` | Boost.Geometry R-tree |
| Web build | wasm-bindgen + wgpu | Emscripten |

The Rust stack is the most coherent single toolchain for native plus wasm. The C++ stack fits alongside
existing C++/wasm work and reuses Google's ink code directly.

## Decisions

- **Core language: Rust** (owner decision, 2026-11-10). Both CRDT candidates (Loro, yrs) are Rust libraries, so the
  core calls them directly instead of through C bindings. Vello on wgpu renders natively and on WebGPU from one
  codebase, and wasm-bindgen is a simpler web build than Emscripten. What C++ offered was direct reuse of google/ink;
  ink-stroke-modeler stays reachable from Rust through FFI if a port of perfect-freehand is not good enough. The C++
  column above is kept as the record of the alternative. Reopen the question only if the spike shows that neither
  the perfect-freehand port nor ink-stroke-modeler through FFI gives good enough strokes.
- **License: MIT OR Apache-2.0** (owner decision, 2026-11-10). It is what most Rust libraries use, so people embedding
  Everink already know its terms, and the Apache half carries a patent grant. Copyleft would have kept forks open
  but works against goal 4 (library first).

## Open questions

1. ~~Rust or C++ for the core?~~ Decided: Rust (see Decisions).
2. ~~Loro or yrs?~~ Decided on 2026-11-12: yrs, and only for the text inside text objects, because Everink holds
   its own object map (see `docs/spike/results-2b-text-libraries.md` for the trial of Loro, yrs and Automerge).
3. How to shard a CRDT document by tile without breaking cross-tile objects such as long strokes? Proposed answer in
   [design.md](design.md): do not shard by tile; the object is the unit of sync.
4. Vector only, or vector plus raster tiles for imported images? The owner wants pasted images and possibly video
   (2026-11-11); [design.md](design.md) proposes them as placed objects, with no raster painting.
5. ~~License: permissive (MIT/Apache-2.0) or copyleft?~~ Decided: MIT OR Apache-2.0 (see Decisions).
6. ~~Which first app proves the library: a whiteboard, a notebook, or a planning surface?~~ Decided: a whiteboard
   (owner, 2026-11-12).

## Next steps

1. Run the spike: one stroke type, two clients, a server, native and web. Its questions are listed in
   [design/README.md](design/README.md); question 2 above is one of them.
2. Write requirements with acceptance criteria for the spike.
3. Measure: stroke latency, merge time, and memory for 100k strokes.
