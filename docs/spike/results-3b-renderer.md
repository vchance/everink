# Spike slice 3b: the renderer, natively

Date: 2026-11-12. Code: `spike/render-trial` (a package of its own, outside the Cargo workspace).
Design: [`../design/06-rendering-and-ink.md`](../design/06-rendering-and-ink.md), "Frame pipeline", "Tile cache"
and "Performance budget".

This is the native half of question 10 of the [plan](README.md): does the renderer carry the load? The browser
half (WebGPU and WebGL2, and the size of the wasm build) is part 3c. Pen latency (question 11) is the owner's.

## Answer so far

On an Apple M4, redrawing a whole 3840 x 2160 view of 10,000 strokes from nothing takes **61 ms with the design's
outline and 400 ms with the outline slice 3a built to replace it**, with the processor renderer; the GPU
renderer takes 78 to 110 ms and 420 to 480 ms. Section 06 names this "the first measurement to make" and gives
its own reading: 60 ms is acceptable (the view is soft for four frames after a large zoom change), 400 ms is not.

- **The cost is on the processor, not the GPU.** `vello_gpu` prepares every path on one processor thread before
  the GPU sees it. That preparation is the same work in both renderers and repeats to within a few percent from
  run to run; it is most of the time for every outline form. Waiting for the GPU adds 23 to 63 ms for the whole
  view, and that part varied about twofold between sessions.
- **The cost follows the number of segments in the outlines**, and slice 3a's robust outline has nine times as
  many as the design's (1,485 a stroke against 165). So the two findings of slice 3 meet: the outline that
  leaves no holes is too heavy for the renderer as it stands, and the one that is light leaves holes.
- **`vello_cpu` is faster than `vello_gpu` here**, in every row and every session. On this evidence the GPU
  renderer does not buy speed for a scene of many small strokes; it may for other content.
- **A view that is not changing costs none of this.** The tile cache exists so that panning and a steady view
  draw cached tiles. That path (composing the view's cached textures) is not built or measured here.

Nothing here is a browser. WebAssembly has one thread and ran slice 3a's arithmetic three to four times slower
than native, so the browser figures will be worse; part 3c measures them.

## What was built

`spike/render-trial` builds a scene of pen strokes (random walks of 30 to 120 points, 2 to 6 pixels wide, each
about 60 to 250 pixels long), makes each stroke's outline with `everink-core`, and spreads them over the view.
It then draws the view the way section 06 does: in tiles of 512 x 512 pixels, each tile a scene of the strokes
that touch it, rendered into a texture. It times that with `vello_gpu` 0.4.0 on the machine's GPU (through
`wgpu` 30.0.2) and with `vello_cpu` 0.4.0 into a pixel buffer, for three forms of outline. As a check, it reads
the busiest tile back from both renderers and compares their coverage pixel by pixel: the pixels where the two
differ by more than an eighth of full coverage must be under two in a hundred of the pixels either drew. In
every run recorded here they differed on none (0 of about 138,000 drawn pixels of a tile that is half covered).
The check needs a GPU, so it does not run on CI.

It is outside the workspace, with its own lock file, for the reason the text trial is: the renderer brings in
about 200 packages.

## Measurements

Machine: Apple M4 (aarch64, macOS; GPU "Apple M4", Metal), native, release build, Rust 1.97.1. Command:
`cargo run --release --locked --manifest-path spike/render-trial/Cargo.toml [strokes]`. The view is
3840 x 2160: 40 tiles. Within a session each figure is the median of five renders of the whole view after one
that is not counted. The trial was run in five sessions on the same machine (four by the author, one by the
reviewer of this slice); the tables give the range of those medians.

### 10,000 strokes (327 a tile on average)

| Outline | Segments a stroke | `vello_cpu`, whole view | ... a tile | `vello_gpu`, whole view | ... a tile |
|---|---|---|---|---|---|
| Discs and bands (slice 3a's `build`) | 1,485 | 394 to 407 ms | 10 ms | 420 to 478 ms | 10.5 to 12 ms |
| Octagons and bands (this trial only) | 889 | 263 to 271 ms | 6.7 ms | 281 to 325 ms | 7 to 8 ms |
| Single path (the design's; leaves holes) | 165 | 61 to 64 ms | 1.5 ms | 78 to 110 ms | 2 to 2.8 ms |

Where the time goes, for the whole view:

| Outline | Building the scenes (processor) | `vello_gpu`'s own work on the processor | Waiting for the GPU | `vello_cpu` rasterising |
|---|---|---|---|---|
| Discs and bands | 386 to 412 ms | 2.9 to 5.0 ms | 30 to 63 ms | 5.5 ms |
| Octagons and bands | 249 to 271 ms | 2.1 to 3.6 ms | 30 to 53 ms | 5.2 ms |
| Single path | 53 to 58 ms | 2.2 to 3.0 ms | 23 to 60 ms | 5.0 ms |

- **Building the scenes** is the calls that hand each stroke's path to the renderer (`fill_path`). In both
  renderers that call flattens the path and makes its coverage strips on the spot, on the calling thread. It is
  the same code for both, and the same time.
- **Waiting for the GPU** is the least steady figure. The trial submits each tile and waits for it before the
  next, so the wait includes that synchronisation as well as the GPU's work; a renderer that submitted several
  tiles at once would wait less. It was 23 to 30 ms in the first session and 28 to 63 ms in the others.
- **Octagons and bands** is the same union as slice 3a's outline with straight edges and half the corners, built
  in plain floats inside the trial. It is there to show how the cost moves with the outline's form; it is not a
  builder to keep, and it is not the same on every machine.
- Building the whole view as one scene instead of 40 tiles takes 367, 244 and 51 ms. Tiles cost about 5% more,
  because a stroke that crosses a tile edge is prepared once for each tile.

### 1,000 strokes (32 a tile)

| Outline | `vello_gpu`, whole view | `vello_cpu`, whole view |
|---|---|---|
| Discs and bands | 54 ms | 41 ms |
| Octagons and bands | 38 ms | 28 ms |
| Single path | 22 ms | 6.8 ms |

With little to draw, `vello_gpu` has a floor of about half a millisecond a tile (20 ms for the view) that
`vello_cpu` does not.

### Against the design's budget

Section 06 budgets, for one frame at 60 Hz: 4 ms of processor time for tile raster (time-boxed) and 3 ms of GPU.
The figures below are the processor renderer's.

| Outline | One tile of 327 strokes | Tiles in a 4 ms box | Frames to redraw the whole view |
|---|---|---|---|
| Discs and bands | 10 ms | none whole | 100 (1.7 s) at 4 ms a frame |
| Octagons and bands | 6.7 ms | none whole | 67 (1.1 s) |
| Single path | 1.5 ms | 2 | 20 (0.33 s) |

The time-box is the design's own and it says "to be tuned"; these are what it gives. A tile cannot be stopped
half-way in this trial, so a tile that takes longer than the box simply overruns it. Where no whole tile fits,
the frames are the view's time divided by 4 ms; for the single path they are 40 tiles at two a frame.

Two things about the comparison with section 06. Its reading of 60 and 400 ms is for a redraw of "about 20
tiles"; a 3840 x 2160 view is 40 tiles of 512 pixels. The total for 10,000 strokes is still comparable, because
the cost follows the strokes' segments and not the number of tiles. The per-tile figures are not: with the same
strokes in half as many tiles, each tile would hold about twice as many and take about twice as long.

## Findings

1. **The renderer's cost for strokes is processor time that grows with the outline's segments, at 20 to 25
   nanoseconds a segment.** (Building the scenes for 10,000 strokes of 1,485 segments, each prepared for 1.3
   tiles, takes 386 ms: 20 ns a segment. For strokes of 165 segments it takes 53 ms: 25 ns.) The GPU is not the
   limit and a faster one would not help. Section 06's budget table has tile raster at 4 ms of processor and
   3 ms of GPU a frame; the measured split of processor to GPU time is about 13 to 1 for discs and bands, 8 to 1
   for octagons and bands, and 2 to 1 for the single path, on the steadiest session.
2. **Slice 3a's outline is too heavy as it stands.** It removes the holes of the single path by drawing a
   sixteen-cornered disc of curves at every point. For the renderer that is nine times the segments and five
   times the time. Three ways out, none tried beyond the octagon figure above:
   - fewer and straighter corners (octagons: a third less time, still 3.6 times the single path);
   - discs only where the stroke turns sharply or its width changes, and bands elsewhere;
   - one path for the stroke's outside, found by a true union of the pieces. That is the polygon clipping
     section 06 chose to avoid, and it would have to be the same on every machine.
3. **`vello_cpu` beats `vello_gpu` on this content.** Section 06 puts the processor renderer last, as "best
   effort" with a banner. On an M4, for many small strokes, it is the faster of the two, and it needs no GPU
   device, no shader compilation and no fallback ladder. Whether that holds for large fills, images and text,
   on slower processors, and on one thread in a browser is not shown here.
4. **A full redraw is far from one frame either way.** 61 ms is four frames at 60 Hz and 400 ms is twenty-four.
   The design does not ask for a redraw in a frame; it asks that redraws be rare (the tile cache) and spread
   (the time-box). Both parts are still to be built and measured.
5. **The scene building is one thread, and tiles are independent.** Natively, several tiles could be prepared at
   once on several threads. A browser page that is not cross-origin isolated has one thread (section 07), so
   this does not help there.

## What this part does not show

- **A browser.** WebGPU, WebGL2, and the size of the wasm build: part 3c.
- **The tile cache and the compose pass:** drawing the view's cached tiles as textures, invalidation, eviction,
  the cross-fade. The claim that a static scene costs "well under 1 ms" is still an estimate.
- **Frame rate while panning or zooming.** "10,000 objects at 60 frames a second" depends on the cache; what is
  measured is the redraw the cache avoids.
- **Other content:** large fills, images, text, clips, and objects cut by slice 3a's cut.
- **Other machines.** The design asks for three reference machines and a mid-range laptop; this is one fast one.
- **Moving many lifted objects**, which section 06 calls "the frame-rate case the spike must measure".
- **Threads**, and `vello_cpu`'s own multi-threaded mode.
- **A GitHub-hosted runner has no GPU.** CI runs the trial with the processor renderer only, at 2,000 strokes,
  to keep it building; its figures are not used, and the comparison of the two renderers does not run there. A
  software GPU device, if a runner ever has one, is reported and not used.

## Where the code departs from the design

1. **Each tile's scene is built from the strokes' whole paths.** Section 06 keeps an `EncodedObject` per object
   and flattens it per tile; the trial hands `vello` the same path for every tile a stroke touches.
2. **The renderer is called directly.** Section 06 puts a `Backend` trait in front of it; the trial has none.

## What the design should change

1. Section 06, "Performance budget": tile raster is processor time, 20 to 25 ns a segment, and the GPU share is
   smaller and less steady; replace the 4 ms and 3 ms split with these figures (Finding 1), and say how many
   tiles a view is (40 at 3840 x 2160, not about 20).
2. Section 06, "Outline algorithm": with slice 3a's correction, choose an outline form by what it costs the
   renderer as well as by whether it leaves holes (Finding 2).
3. Section 06, the backend order: say that the processor renderer beat the GPU one natively for strokes, and
   that the order is to be confirmed by the browser figures (Finding 3).
4. Section 06, "Tile cache": a tile's raster should be able to stop and go on, or tiles should be smaller where
   they are dense; a tile of 327 strokes overruns the 4 ms box whichever outline is used but the lightest.

These are recorded here and made in a separate change.

## Kept afterwards

`spike/render-trial` stays until the renderer question is settled by part 3c and the outline form is chosen.
(Later: the owner chose runs on 2026-11-13, after slice 3d. The trial now draws that form and octagons and
bands; the forms measured in this file are at commit `4b84a65`, where its commands give its tables.)
