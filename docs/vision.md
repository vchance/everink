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

## Open questions

1. Rust or C++ for the core?
2. Loro or yrs? Loro handles movable trees and reordering well; yrs has the larger ecosystem and hosted servers.
3. How to shard a CRDT document by tile without breaking cross-tile objects such as long strokes?
4. Vector only, or vector plus raster tiles for imported images?
5. License: permissive (MIT/Apache-2.0) or copyleft?
6. Which first app proves the library: a whiteboard, a notebook, or a planning surface?

## Next steps

1. Decide questions 1 and 2 with a small spike: one stroke type, two clients, a relay, native and web.
2. Write requirements with acceptance criteria for the spike.
3. Measure: stroke latency, merge time, and memory for 100k strokes.
