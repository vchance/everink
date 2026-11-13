# Spike slice 3, part c: the renderer in a browser, and the tile cache

Date: 2026-11-12. Code: `spike/render-web` (and `spike/render-trial/src/scene.rs`, the scene both trials draw).
This part answers the browser half of question 10 of [`README.md`](README.md): **does the renderer carry the
load?** Part 3b ([`results-3b-renderer.md`](results-3b-renderer.md)) measured it natively and left out the
browser, the WebAssembly size, and the tile cache with its compose pass. Those are here.

Every figure is from one machine and one browser: an Apple M4 (macOS), Chrome 157, its window in front, release
builds, Rust 1.97.1. Safari, Firefox, a phone and a slower machine are not measured.

## Answer

- **A view that is not being redrawn holds 60 frames a second.** With the view's 40 tiles kept as textures and the
  3840 x 2160 canvas composed from them on every frame while it pans, Chrome gave a frame every 16.7 ms on both
  WebGPU and WebGL2: no late frame in seven runs of 300 frames on WebGPU, and one (the first) in three runs on
  WebGL2. The page's thread spent a median of 0.2 to 0.8 ms on a frame (5.2 ms at most).
- **Redrawing tiles is the shortfall, and it is processor time, as natively.** Redrawing the whole view of 10,000
  strokes from nothing takes, in WebAssembly with SIMD, **80 ms with the design's outline and 540 ms with the
  robust outline of slice 3a**; without SIMD, 185 to 210 ms and 1,240 to 1,340 ms. The same redraw natively takes
  61 and 394 ms. So WebAssembly costs **1.3 to 1.4 times native with SIMD and 3.0 to 3.4 times without**.
- **WebGPU, WebGL2 and the processor renderer take the same time**, to within a few percent, because the time is
  the preparation of paths, which all three do on the page's one thread. What was left to wait for once all 40 tiles had been
  sent to the GPU was under 3 ms on either backend.
- **All three draw the same pixels on the tile compared, in colour and coverage.** On the busiest tile, every pair of renderers differs on 0 pixels of about
  138,000 drawn, for every outline form; and the canvas composed from the cache on WebGL2 differs from the tile
  drawn directly on 0.
- **The WebAssembly module is small.** Built for size with one renderer: 476 KB for WebGPU, 364 KB for WebGL2,
  519 KB for the processor renderer; 152, 122 and 155 KB sent with Brotli. All three together: 914 KB, 262 KB
  sent. These builds draw paths and images only: no text and none of the renderer's optional shaders.

## What was built

`spike/render-web` is a package of its own, outside the workspace, with its own lock file, like
`spike/render-trial`. It compiles to `wasm32-unknown-unknown` and has one feature for each renderer:

- `webgpu`: `vello_gpu` 0.4.0 through `wgpu` 30.0.2's WebGPU backend.
- `webgl`: `vello_gpu`'s own WebGL2 backend (it does not go through `wgpu`).
- `cpu`: `vello_cpu` 0.4.0 into a pixel buffer.

For each outline form it draws the view's 40 tiles of 512 pixels six times (the first not counted), each tile
into a texture of its own. The 40 textures are the tile cache. A second scene the size of the view then draws
each cached texture as an image, moved by the pan offset, into the canvas: that is the compose pass, and it uses
the renderer's own support for textures it does not own. `www/index.html` runs all of it, then asks every pair
of renderers whether they drew the same pixels.

The scene is the one part 3b drew: `spike/render-trial/src/scene.rs`, now a file of its own that both trials
build in. The native trial's figures did not change with the move (same-day run below).

## Measurements

Machine: Apple M4, macOS, Chrome 157 (WebGPU adapter "Apple M4"), display 1440 x 932 at a device pixel ratio of
2, 60 Hz. Each canvas has 3840 x 2160 pixels and is shown at 480 x 270. Commands: `sh spike/render-web/build.sh`,
then the page as `spike/render-web/README.md` says. Each figure is the median of five redraws of the whole view
after one that is not counted; a range is over sessions (three with SIMD, two without).

### Redrawing the whole view, 10,000 strokes (327 a tile on average)

With WebAssembly SIMD (`-C target-feature=+simd128`):

| Outline | Segments a stroke | `vello_cpu` | `vello_gpu`, WebGL2 | `vello_gpu`, WebGPU | ... a tile | Native `vello_cpu`, same day |
|---|---|---|---|---|---|---|
| Discs and bands (slice 3a's `build`) | 1,485 | 540 to 542 ms | 536 to 546 ms | 540 to 555 ms | 13.5 ms | 394 ms |
| Octagons and bands (the trials only) | 889 | 358 to 363 ms | 355 to 362 ms | 358 to 360 ms | 9.0 ms | 265 ms |
| Single path (the design's; leaves holes) | 165 | 80 to 81 ms | 76 to 80 ms | 78 to 79 ms | 2.0 ms | 61.5 ms |

Without SIMD:

| Outline | `vello_cpu` | `vello_gpu`, WebGL2 | `vello_gpu`, WebGPU | ... a tile |
|---|---|---|---|---|
| Discs and bands | 1,278 to 1,338 ms | 1,244 to 1,308 ms | 1,245 to 1,345 ms | 31 to 34 ms |
| Octagons and bands | 905 to 970 ms | 888 to 913 ms | 881 to 882 ms | 22 to 24 ms |
| Single path | 206 to 211 ms | 183 to 187 ms | 183 to 185 ms | 4.6 to 5.3 ms |

Where the time goes, for the whole view with SIMD:

| Outline | Building the scenes | `vello_gpu`'s own work on the page's thread | Left to wait for the GPU | `vello_cpu` rasterising |
|---|---|---|---|---|
| Discs and bands | 531 to 546 ms | 2.3 to 6.0 ms | 0.7 to 2.9 ms | 7.5 to 7.7 ms |
| Octagons and bands | 351 to 357 ms | 2.2 to 3.3 ms | 0.6 to 3.0 ms | 6.8 to 7.2 ms |
| Single path | 72 to 75 ms | 1.9 to 3.4 ms | 0.5 to 2.5 ms | 7.1 to 7.5 ms |

- **Building the scenes** is the same call as natively (`fill_path`): it flattens each path and makes its
  coverage strips on the calling thread. Natively it took 389, 259 and 56 ms the same day.
- **Left to wait for the GPU** is one wait after all 40 tiles are sent, not the GPU's whole time: the browser's
  GPU process works while the page prepares the next tile, so this is what it had not finished by the end. It
  shows that the GPU keeps up. On WebGPU the wait is `onSubmittedWorkDone`; on WebGL2 it is a read of one pixel,
  because Chrome's `finish` returns at once. WebGL2 waits longer (1.5 to 3.0 ms) than WebGPU (0.5 to 1.0 ms).
- Part 3b's native "waiting for the GPU" was 23 to 63 ms because it waited after every tile. Waiting once, the
  whole view's GPU work is not visible beside the path preparation.
- Without SIMD, `vello_cpu`'s own rasterising takes 27 to 28 ms for the view instead of 7.5.

### 1,000 strokes (32 a tile)

| Outline | `vello_cpu` | WebGL2 | WebGPU | (all with SIMD) | Without SIMD: `vello_cpu` | WebGL2 | WebGPU |
|---|---|---|---|---|---|---|---|
| Discs and bands | 59 ms | 60 ms | 58 ms | | 132 ms | 128 ms | 131 ms |
| Octagons and bands | 40 ms | 40 ms | 39 ms | | 97 ms | 91 ms | 94 ms |
| Single path | 9.9 ms | 10.8 ms | 11.6 ms | | 23 ms | 19 ms | 21 ms |

One session each; the WebGL2 figures without SIMD leave out the wait for the GPU, which that run did not yet
measure (about 2 ms). The floor that `vello_gpu` showed natively with little to draw (20 ms for the view) is not
there when the wait is once for the view: 10.8 and 11.6 ms against `vello_cpu`'s 9.9.

### The view composed from the cache while it pans

300 frames, each drawing the 40 cached tiles into the canvas at a new offset, on `requestAnimationFrame`.

| Backend | Time between frames: median | 95th | Frames more than half a frame late | Page's thread, a frame: median | 95th | most |
|---|---|---|---|---|---|---|
| WebGL2 (three runs) | 16.7 ms | 17.6 ms | 0, 0 and 1 of 300 (the first frame) | 0.2 to 0.3 ms | 0.4 to 0.5 ms | 0.8 ms |
| WebGPU (seven runs) | 16.7 ms | 17.5 to 17.7 ms | 0 of 300 in every run | 0.6 to 0.8 ms | 0.8 to 1.0 ms | 5.2 ms |

- The time on the page's thread is building the compose scene and handing it to the renderer. The GPU's time is
  not measured apart; that no frame is late shows it fits the 16.7 ms.
- The canvas is 3840 x 2160 pixels but shown at 480 x 270, so the browser scales it down to put it on screen. A
  canvas shown at its full size on a 4K display is not measured.
- The pan moves the same 40 tiles. A real pan also brings new tiles into view, which have to be drawn; that cost
  is the table above (see "Against the design's budget").
- Two earlier WebGL2 runs are not in the table: the page measured them wrongly (it took the first frame's time
  from before the page's thread was free, and showed a gap of many seconds). The page now lets one frame pass
  first. Drawing the same frames by hand showed 16 to 17 ms throughout.

### Do they draw the same thing?

On the busiest tile (half of it drawn on, about 138,000 pixels), the pixels where any of red, green, blue or
coverage differs by more than an eighth of full, among those either renderer drew. The sessions the tables above
come from compared coverage only; the comparison of colour was added in review and run in one more session
(below), with the same result:

| Pair | Discs and bands | Octagons and bands | Single path |
|---|---|---|---|
| `vello_cpu` and WebGL2 | 0 | 0 | 0 |
| `vello_cpu` and WebGPU | 0 | 0 | 0 |
| WebGL2 and WebGPU | 0 | 0 | 0 |
| The canvas composed from the cache on WebGL2, and each of the three | | | 0 |

The same in every session, at 10,000 and at 1,000 strokes. The page fails if any pair differs on two in a hundred
or more. The canvas composed on WebGPU is not read back, so it is not in this check. It was looked at: it shows the same
strokes as the WebGL2 canvas, on black, because its surface is opaque.

### A run on the final code, with a fine clock

After review the page compares colour as well as coverage and is served so that its clock is fine (0.005 ms).
One more session with SIMD at 10,000 strokes, in a fresh Chrome 157 window with an empty profile:

| Outline | `vello_cpu` | WebGL2 | WebGPU | `vello_gpu`'s own work (WebGL2, WebGPU) | `vello_cpu` rasterising |
|---|---|---|---|---|---|
| Discs and bands | 549 ms | 555 ms | 544 ms | 3.2 and 4.9 ms | 8.1 ms |
| Octagons and bands | 366 ms | 361 ms | 365 ms | 2.5 and 4.5 ms | 7.4 ms |
| Single path | 82 ms | 78 ms | 81 ms | 2.1 and 4.2 ms | 7.5 ms |

The pan: no late frame of 300 on either backend; the page's thread took a median of 0.12 ms a frame on WebGL2
(0.24 at the 95th, 1.2 at most) and 0.30 ms on WebGPU (0.58, 1.8). All twelve pairs differed on 0 pixels. So the
figures above stand, and the small ones were not an effect of the rounded clock.

A first attempt at this run is not used: another job was compiling on every core of the machine (load average
72), and the redraw took 1,416 ms where it takes 549. The native trial was slowed the same way at that moment.

### Size and start-up

`sh spike/render-web/build.sh`; sizes of the `.wasm` after `wasm-bindgen` 0.2.130, with no further size tool
(`wasm-opt` was not run).

| Build | Bytes | gzip -9 | Brotli |
|---|---|---|---|
| WebGPU only, for size | 475,782 | 184,677 | 152,289 |
| WebGL2 only, for size | 363,939 | 146,066 | 122,452 |
| `vello_cpu` only, for size | 519,215 | 196,304 | 155,066 |
| All three, for size | 913,903 | 334,247 | 261,878 |
| All three, for speed (the build measured above, without SIMD) | 1,456,827 | 417,175 | 306,475 |
| All three, for speed, with SIMD | 1,345,711 | 389,534 | 289,922 |

- "For size" is `opt-level = "s"`, link-time optimisation, one codegen unit, `panic = "abort"`. Its speed is not
  measured.
- Each module also holds the scene builder and `everink-core`'s outline code, so the renderers themselves are
  smaller than their rows.
- `vello_gpu` is built without its default features: no text, and none of the optional shaders (blurred rounded
  rectangles, sweep gradients, full image support). A product build will have text, so it will be larger.
- Loading and compiling the speed build took 45 to 68 ms from a local server. `vello_gpu` compiled its 8 WebGL2
  shaders in 369 ms the first time the page was opened and in 5 to 18 ms afterwards.

### Against the design's budget

Section 06 budgets 4 ms of processor time a frame for tile raster. One tile of 327 strokes, and what follows:

| Outline | One tile, with SIMD | Tiles in a 4 ms box | Frames to redraw the whole view, at 4 ms a frame | Without SIMD: one tile |
|---|---|---|---|---|
| Discs and bands | 13.5 ms | none whole | 135 (2.3 s) | 31 to 34 ms |
| Octagons and bands | 9.0 ms | none whole | 90 (1.5 s) | 22 to 24 ms |
| Single path | 2.0 ms | 2 | 20 (0.33 s) | 4.6 to 5.3 ms |

Where a tile does not fit the box, the frames are the view's time divided by 4 ms; for the single path they are
40 tiles at two a frame. A pan that brings one new column of five tiles into view costs 10 ms with the design's
outline and SIMD, and 68 ms with the robust one.

## Findings

1. **The tile cache does what the design says it does.** A view composed from cached tiles is cheap on both
   backends: under a millisecond of the page's thread and no late frames. "10,000 objects at 60 frames a second"
   holds for a view that is steady or panning over tiles it has.
2. **The shortfall is the redraw, and it is the outline's segments again.** Part 3b's finding carries over
   unchanged: the cost is path preparation on one processor thread and follows the segment count. In the browser
   only the design's outline fits the tile budget, and only with SIMD.
3. **WebAssembly SIMD is worth 2.3 to 2.6 times here** and is the difference between 1.4 and 3.3 times native. It
   is a build flag, and a module built with it does not load in a browser without it. Section 06 should require
   it or plan two builds.
4. **The backend does not matter for strokes, in Chrome on this machine** (where WebGL2 runs through ANGLE on
   Metal). WebGPU, WebGL2 and the processor renderer are within a few
   percent, and WebGL2 is not a second-class fallback for this content. `vello_gpu`'s WebGL2 backend worked as
   documented, including texture targets and textures it does not own.
5. **Wait for the GPU once for a batch of tiles, not after each.** Part 3b's native GPU figures were dominated
   by its per-tile wait.
6. **The module size is not a concern** at these sizes, before text.

## What this part does not show

- **Other browsers and machines.** Chrome 157 on one Apple M4 only. Safari and Firefox (WebGPU and WebGL2), a
  phone, an integrated-graphics laptop: the owner's to run, or a later slice's. The page needs nothing but a
  static server.
- **Tile invalidation and eviction.** The cache here is 40 textures that never change after they are drawn. Which
  tiles an edit dirties, how many levels of tiles are kept and when they are dropped are not built.
- **Zoom.** Tiles are composed at their own scale. Drawing cached tiles scaled while new ones are made (the
  design's "soft for four frames") is not shown.
- **Redrawing off the page's thread.** WebAssembly here has one thread. The design keeps the core single-threaded
  on the web (section 08: threads need cross-origin isolation). A worker that prepares tiles is not tried.
- **A fine clock for the small figures.** The sessions in the tables were served by a plain static server, so
  Chrome rounded the page's clock to 0.1 ms. The parts are sums of 40 per-tile readings, so the figures of a few
  milliseconds a view (`vello_gpu`'s own work, `vello_cpu` rasterising) and the pan's fraction of a millisecond a
  frame carry about that much noise each. The totals of 80 ms and more do not. `serve.py` now gives the page a
  clock of 0.005 ms, and the page prints which it has.
- **Other content.** Strokes only: no text, images, large fills or clips.
- **GPU time for a composed frame**, apart from the fact that no frame was late; and a canvas shown at 4K.
- **The speed of the size builds.**

## Where the code departs from the design

- As in part 3b: each tile's scene is built from the strokes' whole paths, and there is no `Backend` trait.
- The compose pass draws tiles through the renderer (a scene of images) and not with a pass of its own.
- On WebGL2 a tile texture's first row is the bottom of the image, so the compose scene flips each tile.

## What the design should change

These go into `docs/design/` in a separate change, with part 3b's four.

1. Section 06, "Performance budget": give tile raster in the browser as measured (2 ms a tile of about 330
   strokes with the design's outline and SIMD), and say the budget assumes WebAssembly SIMD.
2. Section 06, the backend order and section 07's build: WebAssembly SIMD is required (or two builds are
   shipped); state which.
3. Section 06, "Tile cache": the compose pass can be the renderer's own image drawing on both backends; the wait
   for the GPU is once for a batch of tiles.
4. Section 06, WebGL2: it is `vello_gpu`'s own backend, not `wgpu`'s, and it performs as WebGPU does for strokes.

## Kept afterwards

`spike/render-web` stays, with `spike/render-trial`, until the renderer and the outline form are chosen.
(Later: the owner chose runs on 2026-11-13, after slice 3d. The page now draws that form and octagons and
bands; the forms measured in this file are at commit `4b84a65`.)
