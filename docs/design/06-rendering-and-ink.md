# Rendering, ink, text and level of detail

Status: detailed design, 2026-11-11, proposal.

Numbers in this document are starting values to test, not measurements, unless a source is given. Claims that could
not be checked on 2026-11-11 are marked "unverified" and collected at the end of the Sources section.

## Scope

This section covers how the surface is drawn and how ink is made:

- the choice of renderer and its fallbacks;
- the frame pipeline, from "what is visible" to a presented frame, and what is cached between frames;
- frame pacing, GPU memory and texture budgets;
- the ink pipeline: pointer input, smoothing, prediction, the variable-width outline, the low-latency path, erasing;
- shapes, arrows and path filling and stroking rules;
- text layout and text drawing;
- anti-aliasing and color;
- level of detail (LOD) under unlimited zoom, including very large objects seen from far below them;
- overview pictures: pre-rendered images of dense regions, how they are made, stored and blended;
- a performance budget per frame stage.

It leaves these to other sections, and states what it assumes of them at the end:

| Topic | Section |
|---|---|
| Positions, camera, exact arithmetic, the cut of an outline to the view | 01 |
| Object schema, order keys, merge, undo | 02 |
| View subscriptions, what the server sends, where pictures are stored | 03 |
| Network side of live ink and cursors | 04 |
| Decoding images and video, font files, the blob store | 05 |
| Toolbars, menus, accessibility, text entry (IME) | 07 |
| Who may do what | 08 |

## The problem

Four requirements pull against each other.

1. **60 frames per second with 10,000 objects in view** (a frame is 16.7 ms). If every object were re-encoded and
   re-rasterized every frame, the budget would be 1.6 microseconds per object, with a 3840 x 2160 pixel target on a
   high-density screen (8.3 million pixels). A GPU vector renderer does not reach that for arbitrary paths while the
   CPU also prepares them. Most frames must therefore reuse earlier work.
2. **Unlimited zoom.** A stroke drawn at one zoom must still be sharp 10^6 times deeper, so ink and shapes stay
   vector and cannot be baked to bitmaps for good. The numbers that reach the GPU are 32-bit floats (about seven
   digits), so they must be small and camera-relative; the exact step belongs to section 01.
3. **Pen to ink under 20 ms.** At 60 Hz one frame is 16.7 ms, so a pen stroke that waits for the next display
   refresh and then for the window system's compositor already spends two to three frames (33 to 50 ms) before any
   prediction. The live stroke needs its own short path, and the 20 ms figure needs a precise definition (see
   [Latency](#latency-what-is-achievable)).
4. **Identical results everywhere.** Two clients must agree on an object's outline (for hit testing, for the exact
   cut in section 01, for text line breaks) even though their GPUs round differently. Only data the program computes
   can be made identical; pixels from different GPUs cannot.

A fifth problem comes from the scale of the surface: a region can hold millions of small strokes, each only a few
pixels across when seen from far above. Sending and drawing them one by one is too slow, so a picture of the region
stands in for them (see [Overview pictures](#overview-pictures)).

## What others do

**Vello and the Linebender stack.** Vello is a Rust 2D renderer on wgpu. It is now three renderers in one repository
([Vello README](https://github.com/linebender/vello)): the original compute-centric renderer (crate `vello`, version
0.12.0 published 2026-11-11), which "remains an experimental implementation for compute-capable GPUs" and needs
compute shaders ([docs.rs/vello](https://docs.rs/vello/latest/vello/)); *Vello CPU*, "the most mature choice"; and
*Vello GPU* (crate `vello_gpu`, 0.4.0 published 2026-11-11, formerly named `vello_hybrid`), which "is intended to
become the primary renderer for production GPU use cases as it matures". Vello CPU and Vello GPU are both built on
**sparse strips**: paths are flattened and sorted into tiles on the CPU, coverage is kept only along path edges
("strips"), solid interiors cost a fixed amount per strip, and the GPU does only the fragment-shader work, so no
compute shaders are needed. Vello GPU therefore has a wgpu backend and a native WebGL2 backend
([ARCHITECTURE.md](https://raw.githubusercontent.com/linebender/vello/main/ARCHITECTURE.md),
[docs.rs/vello_gpu](https://docs.rs/vello_gpu)). The Linebender 2026 Q1 report calls Vello Hybrid "roughly beta
quality" with "rough edges" ([report](https://linebender.org/blog/tmil-25/)). The `vello_gpu` docs list what is
missing: mask layers, complex filter graphs and some blend modes "will panic"; glyph caching is "experimental";
performance "trails the compute-centric `vello` renderer on dynamic, vector-heavy scenes". The v0.3.0 release notes
say it has no API stability guarantee and is not feature-complete against Vello CPU
([release](https://github.com/linebender/vello/releases/tag/sparse-strips-v0.3.0)). Raph Levien's RustWeek 2025 talk
gives the reasons for sparse strips: the compute renderer needs a modern GPU, "fails entirely on WebGL", uses memory
that is high and hard to predict, and is a GPU bottleneck when zoomed far out
([slides](https://docs.google.com/presentation/d/1f_vKBJMaD68ifBO2j83lBly9Zdk-2bsvj_DIHXxvcuk/htmlpresent)). One
public data point (Bunnymark, many small images, an old issue): Vello Hybrid 36 ms, Vello CPU single thread 70 ms,
compute Vello 14 ms ([issue 1427](https://github.com/linebender/vello/issues/1427)). I found no published benchmark
of sparse strips on large path-heavy documents; the spike must produce one.

**Skia (`skia-safe`, CanvasKit).** The most complete and most tested 2D engine. `skia-safe` 0.157.0 (2026-11-03,
MIT) binds native Skia. On the web, CanvasKit is a separate wasm module with its own memory and a JavaScript
interface ([CanvasKit docs](https://skia.org/docs/user/modules/canvaskit/)); a Rust core compiled to wasm would call
into it through JavaScript for every draw, and native and web would run different binding layers.

**tiny-skia.** CPU only, a port of a Skia subset, 0.12.0 (2026-02-02, BSD-3-Clause). Good for tests and a last
fallback; not fast enough at 8 million pixels with 10,000 objects.

**Lyon** (1.0.19, MIT OR Apache-2.0) turns paths into triangles on the CPU; antialiasing then comes from MSAA or
extra geometry. It is simple and works on WebGL2, but every zoom change needs re-tessellation of curves, and
overlapping fills and strokes need extra passes. **femtovg** (0.28.0, 2026-10-27, MIT OR Apache-2.0) is an
OpenGL-style stencil renderer; no WebGPU path and limited text quality. **ThorVG** (MIT, C++) has software, GL,
WebGL and WebGPU backends and about a 170 KB core ([repo](https://github.com/thorvg/thorvg)); it is aimed at icons
and Lottie animation, and using it from Rust means a C++ build in the wasm toolchain.

**Graphite** (Rust, wasm, GPU) is the existing proof that the Rust, wasm and GPU route works for a vector editor; I
could not confirm from its blog index which renderer it uses today. **Zed's GPUI** deliberately writes "a dedicated
shader for each graphical primitive" and found Pathfinder too slow ([Zed blog](https://zed.dev/blog/videogame)):
right for rectangles, text and shadows, wrong for arbitrary user paths. **Rerun** and **Makepad** were not
researched in depth and are not used as evidence.

**Figma** moved its C++ renderer from WebGL to WebGPU through Dawn. Its lessons apply directly: WebGPU readback is
asynchronous; some Windows devices failed in the middle of a session, so Figma starts on WebGPU, falls back to WebGL
when failures occur, and blocklists devices by their fallback rate ([Figma
blog](https://www.figma.com/blog/figma-rendering-powered-by-webgpu/)).

**tldraw** keeps a spatial index (an R-tree), culls shapes outside the viewport, and simplifies small shapes (dashes
become solid, shadows are dropped below a zoom threshold) and picks smaller image versions by on-screen scale
([culling and performance docs](https://tldraw.dev/sdk-features/performance)). Excalidraw's culling was not checked
(unverified).

**Map renderers** (Mapbox GL, MapLibre) draw a pyramid of tiles keyed by zoom, x and y, show a coarser tile while a
finer one loads, and "overzoom" a tile past its last level. MapLibre's own documentation fetched here did not
describe these points, so they are general knowledge (unverified in this session) and the design below takes only
the idea.

**WebGPU in browsers** (state on 2026-11-11, from [web.dev](https://web.dev/blog/webgpu-supported-major-browsers)
and secondary reports): Chrome and Edge since version 113 on desktop and 121 on Android 12 and later with supported
GPUs; Safari 26 on macOS, iOS, iPadOS and visionOS; Firefox 141 on Windows and 145 on macOS; Firefox on Linux and
Android and other Android browsers not shipped. Secondary sources contradict each other on Firefox versions, so the
design must feature-detect and never assume.

**Pen latency.** Chrome's `desynchronized` canvas hint (2d, webgl, webgl2 contexts) skips the usual compositor queue
and in some cases hands the buffer straight to the display controller; no other DOM element may sit above such a
canvas ([Chrome blog](https://developer.chrome.com/blog/desynchronized)). I found no statement that a WebGPU canvas
context accepts the hint (unverified). The Ink API's `DelegatedInkTrailPresenter` lets the OS compositor draw a
trail ahead of the last pointer event; it is marked experimental and not Baseline on
[MDN](https://developer.mozilla.org/en-US/docs/Web/API/DelegatedInkTrailPresenter). `getCoalescedEvents()` is
"limited availability" on [MDN](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents);
`getPredictedEvents()` is Chrome 77, Firefox 89, Safari 18.2 according to an MDN mirror
([w3cub](https://docs.w3cub.com/dom/pointerevent/getpredictedevents), unverified against live MDN). Android's Ink
API (Jetpack, 1.0.0 stable per a third-party blog, unverified) reports 4 ms end to end on a Samsung Tab S8 in its
launch post ([Android
blog](https://android-developers.googleblog.com/2024/10/introducing-ink-api-jetpack-library.html)); an Android
developer's measurement note says "any app can reach" about 16 ms at 60 Hz and 4 ms at 120 Hz
([blog](https://tbuckley-blog-7b3912.pages.dev/writing/stylus-latency), stated targets, not measurements). Apple cut
Pencil latency from about 20 ms to about 9 ms in iPadOS 13 using Metal, prediction and mid-frame event processing
([MacRumors](https://www.macrumors.com/2019/06/21/apple-pencil-latency-ipados-developers) via search summary; 2019
figures). A 50 ms threshold is cited by Chrome as where latency hurts hand-eye coordination.

**Stroke modeling.** *perfect-freehand* (MIT) takes input points, smooths them (`streamline`), simulates or uses
pressure, and returns a closed outline polygon that can self-cross, to be filled; it has no prediction
([repo](https://github.com/steveruizok/perfect-freehand)). Rust ports exist (`perfect_freehand` 0.1.1, MIT, January
2026, a one-person project). *google/ink-stroke-modeler* (Apache-2.0, C++20, needs Abseil) models input as: upsample
to a minimum output rate, a time-windowed wobble smoother, then a spring-and-drag pen tip, and a predictor that runs
the end-of-stroke procedure forward (a Kalman predictor is the alternative)
([repo](https://github.com/google/ink-stroke-modeler)). The wider *google/ink* (Apache-2.0) is the C++ core behind
Android's Ink API; its rendering supports only `android.graphics.Mesh`, so it is no help for rendering here
([repo](https://github.com/google/ink)). A partial, unofficial Rust rewrite, `ink-stroke-modeler-rs`, exists and
omits the Kalman predictor ([docs.rs](https://docs.rs/ink-stroke-modeler-rs)).

**Text.** *Parley* (0.12.0, 2026-10-09, Apache-2.0 OR MIT) does rich-text layout: line breaking, bidirectional text,
selection and cursor utilities. It uses *Fontique* (font lists and fallback), *HarfRust* (a Rust port of HarfBuzz
shaping, 0.16.0, MIT), *Skrifa* (outlines) and *ICU4X* (segmentation, bidi, emoji recognition)
([README](https://raw.githubusercontent.com/linebender/parley/main/README.md)). *cosmic-text* (0.19.0, 2026-04-22,
MIT OR Apache-2.0) is "pure Rust multi-line text handling" with its own layout, bidi support and swash rendering.
Glyph drawing for the Vello family is moving into **Glifo** (formerly `parley_draw`): outline extraction, color
emoji, and an atlas-based glyph cache, "currently in development" ([report](https://linebender.org/blog/tmil-25/)).

## Options considered

Renderer, for drawing vector content to tiles and the screen:

| | A. Vello family: `vello_gpu` (wgpu + WebGL2), `vello_cpu` fallback | B. Skia: skia-safe + CanvasKit | C. Lyon + own wgpu pipelines | D. tiny-skia only (CPU) |
|---|---|---|---|---|
| Language and build | Rust, one toolchain | C++ build for native, separate wasm module for web | Rust | Rust |
| Native and web from one codebase | Yes | No: two bindings, JS bridge on web | Yes | Yes |
| Browsers without WebGPU | WebGL2 backend, then CPU | CanvasKit on WebGL | Pipelines need a WebGL2 port | CPU only |
| Maturity today | Pre-1.0, beta, no API stability, panics on unsupported features | Very high | Mine to write and maintain | Stable, CPU speed |
| Quality of AA, strokes, gradients | Analytic AA, good | Best | MSAA; overlaps and joins need work | Good |
| Text outlines and glyph cache | Yes; cache experimental | Yes | Mine to write | Mine to write |
| Deep zoom | Re-flattened per tile at needed scale | Same | Re-tessellate per zoom | Same |
| Licenses | Apache-2.0 OR MIT | BSD-3-Clause | MIT OR Apache-2.0 | BSD-3-Clause |
| Cost of being wrong | Replace behind our own trait | Large binary, JS bridge | High: a renderer is a product | Too slow for the target |

**Choice: A, behind a small trait, with a raster tile cache in front of it.** The trait is the answer to the
maturity risk: `everink-render` never exposes Vello types to the other crates. ThorVG, femtovg and compute Vello
were not carried to the table's columns: ThorVG and femtovg because of the C++ build and the missing WebGPU path
respectively, compute Vello because it is labelled experimental and cannot run on WebGL2 or in a browser without
WebGPU compute.

What this gives up: Skia's maturity and its complete feature set (filters, blend modes, mask layers). The design
uses only fills, strokes, clip layers, images and normal blending, so the unsupported features are never reached. It
also gives up GPU-only rasterization: sparse strips prepare paths on the CPU, so CPU time per rasterized tile is
real, and the tile cache below exists to keep it off most frames.

Frame strategy, for 10,000 objects at 60 fps:

| | 1. Re-encode and draw everything every frame | 2. Retained scene, transform per frame | 3. Raster tile cache (chosen) |
|---|---|---|---|
| Static scene cost | Full | Full GPU, little CPU | Near zero (compose textures) |
| Pan | Full | Full GPU | Move textures; new edge tiles only |
| Zoom | Full | Full GPU | Scale textures, re-render in the background |
| Sharpness after zoom | Exact | Exact | Exact once re-rendered; softened while rendering |
| Memory | Low | Medium | Tile textures, a budget (below) |
| Risk | Misses 60 fps on dense scenes | Needs per-frame strip work on CPU | Visible softness during fast zoom |

The softness during fast zoom is the same trade map renderers make, and it is shown for a few frames only.

A further renderer candidate named by the owner (2026-11-12) is evaluated in the spike behind the `Backend` trait,
with its details kept outside this repository. Nothing in this design depends on it until it has been measured.

## Design

### Decisions that fix the first release (owner decisions, 2026-11-12)

- **Browsers (owner decision, 2026-11-12).** The WebGPU and WebGL2 paths are supported and tested. The
  processor-only path (`vello_cpu` into a canvas) is best effort: it must open a surface and work, slowly; it is not
  tuned and is not tested as a full experience. CI runs the pixel suite on the wgpu and WebGL2 backends; the
  processor-only backend runs a smoke test (open, draw one of each primitive, pan) and the tests that need it as the
  picture builder.
- **Eraser (owner decision, 2026-11-12).** The whole-object eraser is the default tool. The partial stroke eraser
  (cuts) is a second tool in the toolbar.
- **Text languages (owner decision, 2026-11-12).** Text on the surface supports Latin script only at the first
  release. Other scripts and emoji draw as missing-glyph boxes until support is added. No CJK or emoji fonts are
  shipped or fetched. Bidirectional text, CJK and emoji work is later; the sections below mark it so.
- **First app (owner decision, 2026-11-12).** A whiteboard with about 50 active people on one surface. That puts
  first: shapes and arrows, text boxes (Latin), images, the tile cache with its time-boxed re-render, and drawing
  the cursors, selections and live strokes of 50 people in the overlay without a frame cost that grows with the
  count (one batched pass for all cursors, one name-banner glyph cache). It defers extreme pen-latency tuning: the
  live-ink canvas and the stroke modeler are built to the design below, but the per-device latency measurements and
  prediction tuning wait. Overview pictures stay in build step 6.
- **Controls toolkit (not yet chosen).** The spike builds the same toolbar twice, with Everink's own small toolkit
  and with egui. Either draws through the scene API below, so the choice does not change this section.
- **Merge model (owner decision, 2026-11-12).** Everink's own server-ordered object map, with a CRDT library only
  for text. Nothing in this section depends on it.

**Spike plan, rendering part.** Measure first: the tile raster rate of `vello_gpu` on 10,000 objects (see the
performance budget); the cost of moving many lifted objects; and the stroke path from pen sample to GPU submit. A
further candidate renderer named by the owner is evaluated in the same spike behind the `Backend` trait, with its
details kept outside this repository; nothing depends on it until it has been measured.


### Crate layout

`everink-render` holds everything in this section behind these parts: `Backend` (the renderer trait), `TileCache`,
`FrameBuilder`, `Ink` (modeling and outline), `TextEngine`, `LodPolicy`, and the overview-picture client. The
headless picture builder is a binary in the same crate that links `vello_cpu` and `everink-core`. `everink-core`
owns objects, the index and positions.

```rust
/// The only door to a vector renderer. Implementations: GpuWgpu, GpuWebGl2, Cpu.
pub trait Backend {
    fn capabilities(&self) -> BackendCaps;                       // max texture size, threads, kind
    fn new_tile_target(&mut self, size_px: u32) -> TileTarget;   // 512 x 512 premultiplied RGBA8
    fn render_scene(&mut self, scene: &Scene, into: &mut TileTarget, origin_px: [f32; 2]);
    fn compose(&mut self, frame: &ComposePlan, out: &mut SurfaceTarget); // textured quads, live ink, overlay
    fn upload_image(&mut self, img: DecodedImage) -> TextureId;  // from section 05
}
/// Everything we draw is expressed with these only: fill, stroke, clip layer, image, normal blend.
pub struct Scene { /* paths in object-local f32, transform f64->f32 per object, paint, clip, order index */ }
```

### Scene API for the controls

Section 07's toolbars and menus are drawn through the same renderer and in the same frame as the canvas. The API
they use is this, and nothing else of the renderer is visible to them:

```rust
pub struct SceneFragment { /* encoded paths, glyph runs, images, clips, in screen pixels; cheap to clone */ }
pub struct SceneBuilder;                       // fill/stroke a Path, push/pop clip, draw glyph run, draw image
impl SceneBuilder {
    pub fn fill(&mut self, path: &Path, rule: FillRule, paint: Paint);
    pub fn stroke(&mut self, path: &Path, style: &StrokeStyle, paint: Paint);   // may be screen-constant hairline
    pub fn push_clip(&mut self, path: &Path);   pub fn pop_clip(&mut self);
    pub fn text(&mut self, run: &LaidOutRun, at: [f32; 2], paint: Paint);
    pub fn image(&mut self, tex: TextureId, dst: Rect, src: Rect);
    pub fn finish(self) -> SceneFragment;
}
/// The text service, shared with the canvas: Parley layout over the document font set, never system fonts.
pub trait TextService {
    fn layout(&mut self, text: &RichText, width: Option<f32>, scale: f32) -> LaidOutText;
    fn caret_rect(&self, t: &LaidOutText, index: usize) -> Rect;
    fn hit_test(&self, t: &LaidOutText, p: [f32; 2]) -> usize;
}
```

Controls are retained by `everink-ui`: a widget whose appearance did not change reuses its `SceneFragment`. The
fragment is drawn in the controls layer of the compose pass, uses the same permitted-feature set as the canvas, and
shares the glyph cache. The wgpu device is owned by `everink-render` and handed to the shell at start.

### The frame pipeline

A frame runs these stages. Stages 1 to 4 run only when the camera or the visible data has changed.

1. **Input and camera.** Apply queued input and animation. Read the camera (section 01) and compute a *view change
   record*: moved how far, scale ratio, level crossed or not.
2. **Visible set.** The local index (core) lists the objects crossing the view padded by half a screen, as section
   03 defines it: an object counts when its longer bounding-box side covers at least `eff_min_px` screen pixels. The
   client uses the `eff_min_px` that the server reports in `ViewEnd` (default `min_px` 1), so what the server sent
   and what is culled here are the same set. This runs when the camera leaves the margin of the last query or an
   object arrives or changes, not on every frame of a pan. Tiles listed in `ViewEnd.more` (03) get a marker.
3. **Prepared geometry.** The camera holds a render origin `R` and a cache window (section 01). For each object in
   the window, 01 supplies `Prepared` geometry: either `Plain` (local `f32` path, an `R`-relative origin and a
   per-frame translation and scale) or `Cut` (segments already clipped to the window, relative to `R`, with true
   edges flagged). This section never does exact arithmetic and never sees a `Position`. Order the set by order key
   (one order across levels).
4. **Dirty tiles.** Compare the needed tiles (below) with the cache. A tile is *missing*, *stale* (an object in it
   changed) or *ready*. Missing and stale tiles go on a priority queue, nearest the view center first.
5. **Tile raster, budgeted.** Take tiles from the queue until the budget for the frame (4 ms of CPU, to be tuned) is
   spent. For each tile, build a `Scene` from the objects touching it (cached encoded paths, below) and call
   `Backend::render_scene`. A tile's origin relative to `R` is one small exact subtraction per tile, done by 01's
   helper when the tile is rendered, so the tile's scene holds only small numbers.
6. **Compose.** One GPU pass draws, back to front: tile quads (ready tiles at their exact placement; stale or
   other-scale tiles scaled), live (non-cached) objects, then the live ink, the overlay and the controls. The
   controls arrive as a `SceneFragment` from `everink-ui` (see [Scene API](#scene-api-for-the-controls)).
7. **Present.**

**The tile cache.** A *raster tile* is 512 x 512 device pixels of premultiplied RGBA8 (1 MiB). Section 01 holds the
camera zoom `z` in [0.5, 2048) pixels per unit of the camera's level, with hysteresis, and changes the camera's
level when it leaves that range. The tile octave is therefore defined without reference to which level the camera is
on: `O = 10 * L + ceil(log2 z)`, where `L` is the camera level. Because `z` runs over 12 octaves and a level is 10
octaves, the same `O` is reached from two adjacent levels, and a level change by the camera changes no key. The tile
scale is `2^O` pixels per unit of level 0, equivalently `2^(O - 10 L')` pixels per unit of any level `L'`. For a
cell of level `L'` with `k = O - 10 L'` at least -1 (which holds for every cell the camera can hold, since
`z >= 0.5`), the cell is `1024 * 2^k` pixels, a whole multiple of 512, so tiles never straddle cell boundaries and
line up across cells. A tile is named by the octave and two global whole-number tile coordinates (01's `Cell`, an
integer of any size), so the name does not depend on any camera level; a tile's position relative to the render
origin is the small number computed when it is rendered.

The view shows the tile octave whose scale is at or just above the view's zoom, scaled down by at most a factor of
two with bilinear sampling. When the zoom crosses an octave, tiles of the new octave are rendered in the background
while the old ones stay on screen, then cross-fade over 150 ms. Crossing the camera's level boundary changes nothing
on screen, because the key depends on `O`, not on `L`.

Tiles and the render origin: a tile is rendered from objects prepared relative to `R`, so it is only created for
tiles inside the cache window (half-side 4V), and the tile's pixels are correct whatever `R` becomes later. A
re-base of `R` marks no tile stale. A `Cut` object's pieces are clipped to a window much larger than any tile, so a
tile that is re-rendered after a re-base uses the new cut and looks the same.

```rust
pub struct RasterKey { pub octave: i64, pub tx: Cell, pub ty: Cell }   // 01's `TileKey` is the index's, not this
pub enum TileState {
    Ready { tex: TextureId, built_from: ContentVersion },
    Stale { tex: TextureId },          // shown until replaced
    Picture { tex: TextureId },        // from an overview picture, not yet replaced by vectors
}
pub struct TileCache { map: HashMap<RasterKey, TileSlot>, lru: LruList, budget_bytes: u64 }
```

**What invalidates a tile.** A change to an object (from this client, from sync, from undo) marks stale every tile
touching the object's old or new bounds (bounds from the index, never from the GPU). A font that arrives marks the
tiles of text objects using it. Zoom, pan and re-base alone invalidate nothing: they change which keys are needed.

**Groups.** A group's members are placed relative to the group (section 02), and moving, rotating or scaling a group
is one write to the group. The index gives a member's world box through its ancestors (section 03). When a group's
`Xform` changes, the invalidation is therefore by region, not by member: the tiles touching the group's old world
box and its new world box are marked stale, once. The renderer does not walk the members to find what moved. While a
group is dragged or resized by this user it is *lifted* (next paragraph) as one unit: its members are drawn as one
cached `SceneFragment` (members composed relative to the group by 01's `compose`) under one moving transform, so
dragging a group of 10,000 members costs one transform per frame after the fragment is built, and the tiles are
re-rendered once without the group at the start and once with it at the drop. A remote move of a group is a single
placement change and gets the stale-region treatment, with the tiles refreshed by the normal queue. A child that is
moved on its own inside a group invalidates the tiles under its old and new world box.

**Interactive objects bypass the cache.** The object being dragged, resized or edited, and the objects of a
selection being moved, are *lifted*: drawn live in the compose pass at their moving transform, and omitted from the
tiles (the tiles under them are re-rendered once without them, then again when the move commits). Moving many
separate objects is the worst case: the lifted set is rendered as one scene each frame, and this is the frame-rate
case the spike must measure. If it misses 60 fps, the set is rasterized once to one temporary texture and moved as a
picture until the drop.

**Encoded paths.** For each object the cache keeps `EncodedObject { version, path_local: Arc<[PathEl]>, paint }` in
object-local units, converted from 01's prepared local `f32` path. It is rebuilt only when the object changes (not
when its `Xform` changes). Per tile, the object is emitted with a transform only; nothing is re-flattened until
`render_scene`, which flattens to a tolerance of 0.25 px at the tile's own scale (so deep zoom stays smooth at any
depth). A `Cut` object is emitted as the given segments; only true edges are stroked.

**Static scene cost.** A static, fully cached view costs one compose pass of about 20 textured quads: well under 1
ms of GPU time and no CPU path work. When nothing changes and no animation runs, no frame is produced at all.

### Frame pacing

- Redraw is requested only when something changed: camera, tile ready, remote update, cursor or selection move,
  animation (fade, video frame), or the local live stroke. Idle is zero GPU work.
- On the web the frame callback is `requestAnimationFrame`; native uses the window system's redraw event. The shell
  (section 07) provides a `request_frame()` call. Pointer events are never drawn from their handler; they enqueue
  and the next frame consumes them, except the live-ink path below.
- The target is the display's refresh rate (60 or 120 Hz), not a fixed 60. If a frame's work overruns, the tile
  budget for the next frame is lowered, never the compose pass (it is cheap and keeps the screen responsive).
- During a continuous zoom or pan the tile queue is *time-boxed* (stage 5), so the frame rate holds and sharpness
  catches up over the following frames.
- Hidden tabs and minimized windows: no frames. Video (section 05) and animations ask for frames themselves.

### GPU memory and textures

Starting budgets, shared by tiles and image textures and enforced by LRU eviction (nothing visible is evicted):

| Platform | Tile + image budget | Notes |
|---|---|---|
| Desktop | 512 MiB | 512 tiles |
| Web | 256 MiB | browsers can lose the GPU under pressure |
| Mobile (iPad, Android) | 128 MiB | |

A full-screen 3840 x 2160 view needs about 4.5 x 2.5 = 12 to 20 tiles per octave, 40 MiB with a second octave cached
for zooming. Image textures (decoded in section 05) are counted against the same budget; each image keeps one
texture at the resolution the zoom needs (section 05 supplies the mip levels, section 06 chooses the one for the
current on-screen size with `steppedScreenScale` logic like tldraw's), and an image bigger than the device's maximum
texture size is drawn in tiles. WebGPU's default 2D texture limit is 8192 pixels per side (spec default; read the
real limit from the adapter). Eviction order: tiles of other octaves first, then tiles far from the view, then
images not in the view. If texture creation fails, halve the budget, evict, and retry once; a second failure is
reported as a device problem (see failure modes).

### Backends and fallbacks

At startup the shell asks for a backend in this order, and `Backend::capabilities()` reports the result.

1. **`vello_gpu` on wgpu**, native (Vulkan, Metal, DX12) and in browsers with WebGPU.
2. **`vello_gpu` WebGL2**, in browsers without WebGPU. Cargo feature `webgl`.
3. **`vello_cpu`** into an RGBA buffer shown through a canvas, for browsers or machines with neither (best effort,
   owner decision 2026-11-12: it must open and work, slowly; it is not tuned or tested as a full experience). It is
   single-threaded unless the page is cross-origin isolated (needed for shared-memory threads in wasm); expect a
   lower tile rate, shown as longer softness after zoom. This is a degraded mode, with a banner in the UI.

No other section may assume WebGPU is present. What each backend does for the two paths that touch the browser:

- **Video frames** (section 05 supplies the frame source). WebGPU: the frame is imported with
  `importExternalTexture` and sampled in the compose pass. WebGL2: the frame is uploaded to a texture from the video
  element each time a new frame is ready. CPU backend: the main canvas is a 2D canvas, and the frame is drawn with
  `drawImage` from the video element in the compose step. In all three, a video is a *live* object composed in the
  compose pass, never baked into a tile, because its pixels change; objects with a higher order key that overlap its
  rectangle are drawn live too (omitted from the tiles under it), so the order stays right. If more than 200 objects
  would be promoted that way the video is drawn at the top of its overlap region and the order error is accepted.
- **Live-ink canvas.** It is a separate Canvas2D with `desynchronized: true` on top of the main canvas (below), so
  it is the same under all three backends: the main backend plays no part in the pen's path. Under the CPU backend
  both canvases are 2D canvases, and the live-ink canvas is the only one that is desynchronized.

As with Figma, a WebGPU session that loses its device mid-session falls back to WebGL2 and the device class is
remembered locally for the next session, so a bad GPU is not retried each time. The fallback ladder is tested by
forcing each backend in CI.

Version pinning: `vello_gpu`, `vello_cpu` and `wgpu` are pinned to exact versions and moved together, because
`vello_gpu` has no API stability. Because unsupported features panic, `Scene` construction only uses the permitted
set, and a test draws one of every permitted primitive on every backend.

### Color and anti-aliasing

- Colors are stored as sRGB 8-bit per channel plus 8-bit alpha (non-premultiplied in the document, premultiplied in
  tiles). Blending is done in the sRGB-encoded space, which is what browsers' 2D canvas and most whiteboards do, so
  a 50% black stroke over white looks as people expect. Linear-light blending would be physically better for
  gradients and wrong for the look users know. Whether Vello's default blending space matches this was not verified;
  the test below pins it.
- Edge anti-aliasing is analytic coverage from the renderer; MSAA is not used. Tiles carry premultiplied alpha, and
  the compose pass uses premultiplied "over", so the seams between tiles are exact (edges lie on whole pixels).
- The display surface is sRGB. Wide-gamut (Display P3) is not in the first version.

### Ink

#### Input

The shell delivers `PenSample { x, y, pressure, tilt, azimuth, time, source_flags }` in device pixels, with `time`
in milliseconds from a monotonic clock, and flags `coalesced` (an intermediate sample), `predicted` (a platform
guess), `real`. Rules:

- Web: use `getCoalescedEvents()` where present (to recover the samples browsers merge into one `pointermove`; if
  absent, use the single event), and `pointerrawupdate` where present for lower delay. `getPredictedEvents()` is
  read but not used by default (see Prediction).
- Native: use the highest-rate stream the OS offers (Apple: coalesced touches; Android: historical `MotionEvent`
  samples; Windows: pointer history). The shell converts to `PenSample` and does nothing else.
- Mouse and fingers have no pressure; the width then comes from speed (below).
- Samples go to the stroke modeler as they arrive, not once per frame.

#### Stroke modeling

Choice: **a Rust implementation of the ink-stroke-modeler algorithm (Apache-2.0, credited), not perfect-freehand,
and not an FFI binding.** Reasons: perfect-freehand has no prediction and a simple exponential smoother, but its
outline step is useful and is replicated below; the C++ library needs Abseil and C++20 and does not build to
`wasm32-unknown-unknown` without extra toolchains; the existing Rust rewrite is partial and one person's project.
The algorithm is small (upsampling, wobble average, a spring and drag integrator, end-of-stroke predictor).

```rust
pub struct ModelParams {            // starting values from the C++ defaults, to be tuned by hand in the spike
    pub min_output_rate_hz: f32,    // upsample raw input to at least this rate (about 180)
    pub wobble_timeout_ms: f32, pub speed_floor: f32, pub speed_ceiling: f32,
    pub spring_mass: f32, pub drag: f32,
    pub end_stop_distance: f32, pub end_max_iterations: u32,
}
pub struct StrokeModeler { /* state, ring buffer of last N samples */ }
impl StrokeModeler {
    pub fn push(&mut self, s: PenSample, out: &mut Vec<ModeledPoint>);   // 0..n new points
    pub fn finish(&mut self, out: &mut Vec<ModeledPoint>);               // settles the tip
    pub fn predict(&self, horizon_ms: f32, out: &mut Vec<ModeledPoint>); // never stored
}
pub struct ModeledPoint { pub x: f64, pub y: f64, pub pressure: f32, pub t_ms: f32 }
```

The modeler runs **on the authoring client only**, in screen pixels at the zoom of drawing, using ordinary floats.
Its output is *not* required to be reproduced by other clients. What other clients receive and what is stored is the
modeled result after **thinning and quantizing** (next paragraph), so the modeler can be tuned and its
floating-point use ignored for determinism. This is the key choice that makes "bit-identical across platforms"
cheap: determinism is needed only from the stored points onward.

**What is stored (section 02 holds the field; this section defines its format).** Points are converted to units of
the stroke's own level relative to the stroke's **anchor** and rounded to a fixed-point lattice of 2^-16 unit as
`i32`; pressure is one byte; there is no time. A point is dropped if it lies within 0.15 px (at the drawing zoom) of
the line between its neighbors (thinning), so a typical stroke is 10 to 40 bytes per 100 px of path; the exact
numbers are measured in the spike. A stroke has at most 20,000 points (`MAX_STROKE_POINTS`, section 08's table).

**The stroke rule.** This section owns it, and sections 01, 02 and 04 cite it. Four terms. The stroke's **level** is
the camera's level at the moment the pen goes down, and it does not change while the stroke is drawn, even if the
view auto-pans or the camera crosses a level. The **pen-down point** `P0` is the exact `Position` of the first
sample. A **lattice step** is 2^-16 of a unit of the stroke's level. The **center** `c` is defined below.

1. **Points relative to the pen-down point.** Each sample becomes `q = (x, y)`, integers in lattice steps from `P0`
   on each axis: the exact difference of positions (01's `Position::delta`), scaled by 2^16 and rounded with
   `round_ties_even` after the range check. The first point is `(0, 0)`.
2. **The end-of-stroke limit.** If a new sample would give `|q| >= 2^30` steps (2^14 units) on either axis, the tool
   ends this stroke at the previous point and starts a new stroke whose pen-down point is that previous point, so
   the two strokes join without a gap. 2^14 units is 8,192 pixels at the lowest in-level zoom (0.5 pixels per unit).
   Every stroke therefore has `|q| < 2^30` steps for all its points.
3. **The extent.** Let `min` and `max` be the per-axis minimum and maximum of the `q` values. Then
   `max - min <= 2^31 - 2` steps, under 2^15 units. The stroke's box is `[min - g, max + g]` with `g` the largest
   half width in steps, rounded up (the brush's width is bounded, so the grown box stays inside section 01's size
   band of 2^16 units); that grown box is what `ExtentOracle` returns.
4. **The center.** `c = (min + max) >> 1` per axis, an arithmetic shift in `i64`, which is a floor. Because the
   growth `g` is added on both sides it cancels, so `c` is also the center of the grown box. The final anchor is
   `A = P0 + c * 2^-16` units. This is an exact position, since 2^-16 is a multiple of the position step 2^-54. It
   is placed in the stroke's `Xform`.
5. **Points relative to the final center.** Each stored point is `p = q - c`, in `i64` and then `i32`. For every
   axis, `|p| <= 2^30 - 1`. The worst case is the top of the range: with `min = -(2^30 - 1)` and `max = 2^30 - 2`,
   `min + max = -1`, so `c = -1` (the floor), and `max - c = 2^30 - 1`. In general `max - c = ceil((max - min) / 2)`
   and `c - min = floor((max - min) / 2)`, both at most `2^30 - 1`. So `p` fits an `i32` with a factor of two to
   spare, and the same is true of the first point `(0, 0) - c`.
6. **Deltas.** The bytes store the difference between consecutive stored points, the first from `(0, 0)` (the
   anchor). Two points can be as far apart as `2 * (2^30 - 1) = 2^31 - 2` steps, which still fits an `i32` but needs
   a zigzag value up to 2^32 - 3, a five-byte varint. The decoder therefore reads each delta as a zigzag varint of
   at most five bytes into an `i64`, adds it to the running point in `i64`, and refuses the stroke if any resulting
   point does not fit an `i32`. A stroke that fails this check is refused by the validator (section 08 lists the
   point count limit; this is the range limit).

The result: a stroke is small enough that its points, its extent and its deltas fit with room to spare. The rule
also covers a stroke drawn while the view auto-pans, because the limit is measured from `P0`, in the stroke's own
level, not in screen pixels.

**Preview and final stroke coincide exactly.** While the stroke is drawn, its points (and section 04's remote
preview) are the `q` values of step 1, relative to `P0`. When the pen lifts, steps 3 to 5 compute `c` and the stored
points `q - c`. Nothing is resampled or rounded again, so each final point is the same world position as its preview
point (`P0 + q * 2^-16` equals `A + p * 2^-16` exactly), the outline built from the final points is the same outline
moved by an exact integer, and the swap from preview to object (section 04) shows no shift. Points are immutable
once the pen lifts.

```rust
pub struct PackedPoints { pub count: u32, pub bytes: Arc<[u8]> }   // field of the Stroke object (section 02)
// bytes: for each point, zigzag-LEB128 varints (at most 5 bytes each) of dx, dy: lattice steps from the previous
// stored point, the first from (0, 0) = the anchor. Decoded as i64; every resulting point must fit an i32 (the stroke
// rule). Then one pressure byte (0 = no pressure given, 1..=255). Decoding is integer-only.
pub struct StrokeBrush { pub brush: BrushId, pub base_width: Fixed32, pub style_ver: u8 }  // style block
```

`style_ver` names the outline algorithm version, so a later algorithm never alters old strokes.

#### The outline: variable width, deterministic

The outline is a *closed filled path* made from the stored points. Every client computes it with the same code and
gets the same numbers, which the core's exact cut (section 01), hit test and eraser use.

Algorithm (version 1, named by `style_ver` so later changes never alter old strokes):

0. Decode the points and cuts. Cuts (section 02) are an add/remove set of `Cut { from, to }` over sample parameters
   `StrokeParam { index, t }`, `t` a 16-bit fraction toward the next sample. Sort them, merge overlapping ones with
   integer comparisons, and take the complement: the *runs* of the stroke that remain. A stroke with no cuts has one
   run. A cut end inside a segment adds a synthetic point `p_i + (p_{i+1} - p_i) * t / 65536` computed in `i64` with
   ties to even, and the same for pressure. Each run is built separately by steps 1 to 7.
1. Convert lattice points to `f64` (an exact conversion).
2. Width at each point: `r_i = base_width * w(pressure_i, speed_i) / 2`, where `w` is a 256-entry table of constants
   written into the source (not computed) and linearly interpolated with `+ - * /` only. Speed is used only when
   pressure is unavailable (flag in the brush), taken from point spacing with fixed `t`.
3. Unit tangent and normal at each point from the neighbors with `+ - * /` and `sqrt` (the correctly rounded square
   root; both native and wasm give identical results).
4. Left side forward, right side backward: `p_i +- r_i * n_i`. Where the turn is sharper than a fixed limit, add a
   round join using a fixed table of eight precomputed unit vectors (written as literals) rotated into place with
   multiplications, never `sin` or `cos`.
5. Caps: at the true ends of the stroke, round (same table) or a taper over the first and last `k` points
   (perfect-freehand's idea). At a **cut end** the run ends with a flat (butt) cap perpendicular to the tangent at
   the cut, with no taper, so an erased stroke looks cut, not capped with a pen end. Every run is its own closed
   sub-path.
6. Smoothing: the polyline of offset points is turned into quadratic Béziers through successive midpoints (control
   point = the vertex), which is the standard way to get a C1 curve and the same trick perfect-freehand uses.
   Midpoints are exact in binary arithmetic.
7. **Round onto section 01's lattice.** Every point and Bézier control point of the result is converted with
   `(v * 2^40).round_ties_even() as i64` (`v` in units of the object's level), after the range check, into section
   01's `Outline { segs, closed, fill }` of `Seg::Line` and `Seg::Quad` over `LPt`. A stroke with several runs has
   one `Outline` with several closed sub-paths. This lattice outline, not the `f64` intermediate, is what section
   01's cut consumes, what the index derives bounds from, and what the bit-identical test compares.
8. The path is filled with the **non-zero** rule, so self-overlaps in tight turns need no clipping step (the
   perfect-freehand README suggests a separate polygon-clipping crate; the renderer's non-zero fill makes that
   unnecessary).

The same function, with an *incremental* entry point, builds the live preview and the final stroke (section 04 needs
both to match):

```rust
pub struct OutlineBuilder { /* params, run state, last emitted point */ }
impl OutlineBuilder {
    pub fn new(brush: &StrokeBrush, cuts: &[Cut]) -> Self;
    pub fn push(&mut self, p: InkPoint, out: &mut Vec<Seg>);   // appends only the new segments (stable prefix)
    pub fn finish(self, out: &mut Vec<Seg>) -> Outline;        // closes caps; equals a one-shot build of all points
}
```

`push` never changes segments it has already returned except for the last two, which are returned again; a test
checks that incremental and one-shot builds of the same points give byte-identical `Outline`s.

**Why this is bit-identical.** The rules for outline code are section 01's float rules, and **section 01's list is
the reference**: if the two ever differ, 01's wins and this section changes. Restated here only so the module is
readable on its own:

- Allowed: `+ - * /` on `f32` and `f64`, `sqrt`, `abs`, negation, `copysign`, comparisons, `floor`, `ceil`, `trunc`,
  `round_ties_even`, conversion with `as` (saturating, NaN becomes 0), `f32` to `f64` conversion, bit operations on
  floats, integer arithmetic.
- Banned: `mul_add`, every transcendental function (`sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `powf`, `powi`,
  `cbrt`, `hypot`), float `min`, `max` and `clamp` (write `if a < b`), relaxed SIMD, and fast-math flags.
- No NaN or infinity enters: every input is finite and bounded, and the last step checks `is_finite()` and the
  lattice range and refuses the object otherwise.
- Tables are literals. Fixed-size loops, no parallel reduction.
- `f64::sqrt` is deliberate: it is correctly rounded on every conforming processor, native and wasm.

Platform math differences never touch the stored points, which are integers, so the only risk is in this one small
module. Rasterization of the outline (pixels) is allowed to differ by a pixel level between GPUs.

At extreme zoom the outline is a smooth curve through the stored points; it is only as detailed as the pen sampling
at the drawing zoom. Zoomed 1000 times in, the stroke edge is a clean curve, but it will not show detail the pen
never recorded. This is the honest meaning of "vector stays sharp".

#### Prediction

Predicted points exist only in the **live** layer and are discarded on every new sample; they are never stored,
never sent as part of an object, and never in a presence packet (section 04 sends only real points, so other users
do not see guesses).

- Source: `StrokeModeler::predict`, using the end-of-stroke procedure of the ISM predictor. This is identical on
  every platform and tunable. Platform predictions (`getPredictedEvents`, Apple predicted touches, Android's
  predictor) are an option to compare in the spike; they are not the default because they differ by OS.
- Horizon: up to the expected delay of the output path, capped at 25 ms ahead and shortened toward zero when the pen
  decelerates. On the 60 Hz web path the delay is about 2 to 3 frames (see latency section), so prediction hides
  some but not all of it. The cap keeps wrong guesses (a sudden stop or turn) to a few pixels.
- On the Chrome delegated-ink path, the OS draws its own trail, and the W3C TAG review recommends not predicting on
  top of it. When `DelegatedInkTrailPresenter` is used, our prediction is switched off.

#### Latency: what is achievable

Define terms. **Software latency** is the time from a pointer event's timestamp to the GPU commands that draw it
being submitted. **Pen-to-ink latency** is from the pen touching to the light changing, and includes the digitizer,
the operating system, the compositor and the display scan-out, which the software cannot change. The high-level
design's "under 20 ms of software latency" is read as: **pointer event to GPU submit under 4 ms, and measured
pen-to-ink latency reported with a 240 fps camera on named devices.** The sources show what the platform can do:

| Path | Evidence | Expectation |
|---|---|---|
| Android, native, front-buffer rendering plus prediction | 4 ms end to end on a Samsung Tab S8 (Google launch post) | the best case |
| iPadOS, Metal plus predicted touches | about 9 ms (Apple 2019, iPad Pro) | similar |
| Web, plain `requestAnimationFrame` on 60 Hz | no source found | my estimate: 2 to 3 frames, 33 to 50 ms before prediction (unverified) |
| Web, `desynchronized` 2d/webgl canvas | Chrome documents the shorter path but gives no numbers | better than plain by up to one compositor frame; to be measured |
| Web, delegated ink trail (Chrome) | experimental, no numbers found | OS draws ahead of the last event; measure |

Design consequences:

- **Native shells** (`everink-native`) render the live ink layer straight into the swapchain with latency of one
  frame (`desired_maximum_frame_latency = 1`) and, while inking, request the lowest-delay present mode the platform
  allows. On Android a shell may use the platform's front-buffered renderer for the live layer; that is shell code
  and does not change the core.
- **Web** has two canvases stacked: the main canvas (WebGPU or WebGL2, all but live ink) and above it a **live-ink
  canvas**. The web shell creates that canvas element (a 2D context with `desynchronized: true`, stacked above the
  main canvas) and hands it over through `SurfaceHost::live_ink_target()`; this section draws into what it is given
  and creates nothing in the page. The live stroke is a small polygon: its outline (the same deterministic function,
  run on the modeled and predicted points) is filled with `Path2D` by the browser. That leaves the heavy renderer
  off the pen's path. The cost is a different rasterizer than the final stroke, so edges differ by a fraction of a
  pixel at hand-over; to hide it the live stroke stays up until the tile containing the committed stroke is ready,
  then the canvas is cleared in the same frame the tile appears. Because no DOM element may sit above a
  desynchronized canvas, the accessibility mirror (section 07) must sit *below* both canvases in stacking order;
  this is a requirement placed on section 07.
- If the browser reports `desynchronized` unsupported (check `getContextAttributes()`), the live-ink canvas is still
  used but is composited normally.
- Chrome's delegated ink trail is used as a progressive enhancement when `navigator.ink` exists, with our prediction
  off; its value is a spike question, since it is experimental.
- Pointer events on the web are processed in the event handler for the live layer (modeler, outline, fill the small
  polygon of changed segments only) and queued for the rest. A live stroke is drawn incrementally: only the new
  segments plus the predicted tail are redrawn each time, with the predicted tail erased by redrawing a clip of that
  area; at pen speed this is a few dozen path points.

#### Brushes, color and pressure

Brush parameters (width, minimum width fraction, pressure curve id, taper, opacity) are in the object's style.
**Highlighter** strokes use multiply-like behavior only if the renderer supports the blend mode; `vello_gpu` lists
some blend modes as unimplemented, so highlighter is drawn as normal blending with 40% alpha and a *single path for
the whole stroke*, which avoids darker overlaps within one stroke by the non-zero rule.

#### Erasing

Both erasers test the eraser path against stroke geometry with the deterministic code, so the results are the same
on every client. The whole-object eraser is the default tool and the partial eraser is a second tool (owner
decision, 2026-11-12); this section supplies both.

- **Object eraser**: sets `alive = false` on every object whose outline the eraser path touches. Hit test: the
  eraser polyline inflated by its radius against each candidate's lattice outline (candidates from the index). One
  undo step per gesture.
- **Partial eraser** (section 02 owns the data): points stay immutable. For each stroke touched, the tool finds the
  parameter ranges where the distance from the centerline polyline to the eraser path is less than
  `eraser_radius + half the local width`, rounds each range outward to the `(index, t)` grid (so a hit is never left
  as a sliver), and emits one `CutAdd` per range. Two users erasing different parts of one stroke both take effect,
  because cuts are a set. Undo removes the cuts that gesture added, by tag. A range that covers the whole stroke
  becomes `alive = false` instead. At 64 cuts, section 02 replaces the stroke by pieces; the tool supplies the
  pieces as the runs from step 0 of the outline (their points and the synthetic end points, brush and order key
  kept).
- **Drawing a stroke with cuts.** The outline of the *remaining runs*, one closed sub-path per run, with flat caps
  at cut ends and the brush's caps at true ends (outline steps 0 and 5). Cuts are not drawn as shapes; the erased
  parts are simply absent from the outline, so the exact cut, hit testing and the tiles all see the remaining runs.
  When a cut is added or removed the stroke's outline is rebuilt and the tiles under its old and new bounds are
  stale.
- **Hit-testing a stroke with cuts** (pointer, selection, eraser): against the centerline polyline of each remaining
  run, with the radius of the local half width plus the tolerance of 0.5 px, in the prepared space of section 01. A
  pointer over an erased gap does not hit the stroke; it can still hit an object beneath. The box the index holds
  comes from `ExtentOracle` (below), which ignores cuts and so still covers the whole stroke; a gap therefore never
  makes the box too small.
- No pixel eraser: the surface is vector, and Everink has no raster layer.
- Live feedback: objects and stroke parts the eraser would remove are drawn faded in the overlay until the pen
  lifts.

### Shapes, arrows and path rules

Shapes keep exact geometry in the object schema (a rectangle is a size and corner radii; section 02 owns the
fields). The core turns that into a path of lines and Béziers using the same arithmetic rules as ink:

- Ellipse and rounded corners: four cubic Béziers per quarter using the constant `0.5522847498307936` written as a
  literal. Corner arcs likewise. Rotation applies the stored matrix, no angles.
- Arrow heads: a filled triangle or open chevron computed from the unit direction at the line end (`sqrt` and
  division). The head size scales with stroke width, not with zoom.
- Fill rule: **non-zero** by default; **even-odd** is a style flag for paths with holes (for example, imported or
  boolean-combined shapes).
- Stroke: width in units of the object's level, scaling with zoom like all geometry. Joins: miter (limit 4), round,
  bevel; caps: butt, round, square. Dashes are applied by the renderer, in object units, and are therefore not
  bit-identical across platforms (see below). A *screen-constant* hairline style exists only for overlay and
  controls, never for content.
- Clients agree on the *geometry* (the centerline path, filled outline of ink, arrowhead) bit for bit. Stroke
  outlines of shapes (borders) are produced by the renderer in float and may differ at the pixel level between
  platforms; hit testing therefore uses the centerline plus half the stroke width with a tolerance of 0.5 px, never
  the rendered pixels.

### Text

**Choice: Parley (Fontique, HarfRust, Skrifa, ICU4X) for layout and shaping; Glifo or our own path cache for
drawing.** It is the only one of the candidates that is the same stack as the renderer (the Vello family uses Skrifa
for glyph outlines), is actively released (0.12.0 on 2026-10-09), has rich-text runs, line breaking, bidirectional
text and cursor and selection helpers, and uses ICU4X for Unicode rules. cosmic-text (0.19.0) is a sound alternative
with its own layout; I prefer Parley for shared maintenance with the renderer and for HarfRust's HarfBuzz parity.
The swash/HarfRust stack is what Parley and Vello already use, so choosing Parley avoids integrating them by hand.

How it is used:

- **Layout is done once at the object's own scale, not per zoom.** Text is laid out in units of the object's level
  (for example 16 units for a 16-unit font size) with hinting off, so line breaks never change when the viewer
  zooms. Positions are `f32` offsets from the text box origin (at most a few thousand units, so well within `f32`).
- **Fonts come only from the document's font set**, never from the system: Fontique's system enumeration is
  disabled. Font files are bytes from section 05. A text run names a font by id; the fallback chain per script is
  part of the document font set, also never the OS. At the first release the set holds Latin only (owner decision,
  2026-11-12): a character the fonts do not cover is drawn as the font's missing-glyph box, and no CJK or emoji font
  is shipped or fetched.
- **Rich runs**: font, size, weight, italic, color, underline, strikethrough per run (from section 02's text model).
  Alignment per paragraph. Wrapping at the box width, or no wrap.
- **Bidirectional text** and line breaking: UAX #9 and UAX #14 through ICU4X. Shaping through HarfRust, including
  ligatures. Line breaking and Latin shaping are in the first release; bidirectional text and the other scripts are
  **later**.
- **Emoji (later; draws as missing-glyph boxes at the first release)**: a shipped color emoji font; vector color
  glyph tables where Glifo supports them. Whether Glifo handles the chosen font's format (COLRv1) is unverified; the
  fallback is one bitmap-font emoji set, which is sharp only to its bitmap size (an emoji can look soft at 100x
  zoom, accepted).
- **Determinism**: shaping and layout use `f32` arithmetic and ICU4X data compiled in. Whether Parley's results are
  bit-identical between native and wasm is *unverified*. The requirement is weaker than ink's: the cases that matter
  are line-break positions and cursor placement. Mitigations: golden tests that compare line breaks and glyph
  positions (rounded to 1/64 unit) between native and wasm CI for a corpus of Latin text at first (Arabic, Hebrew,
  Devanagari, CJK and emoji are added when those scripts are); layout widths compared after rounding to 1/64 unit
  before a break decision, so tiny float differences cannot flip a break. If the golden test finds real differences,
  the fall-back plan is for the authoring client to store explicit line-break offsets with each text block (a change
  to section 02).
- **Drawing**: each glyph is an outline (Skrifa) cached per `(font, glyph, variation)` in font units, emitted as a
  filled path with the layout transform. Because tiles are re-rendered only when dirty, the glyph cache needs only
  to avoid recomputing outlines; Glifo's atlas for small text is an optimization to adopt when it is released, not a
  requirement. At extreme zoom a glyph is simply a large vector shape, flattened at tile-render time with the same
  tolerance as any path, so letters stay sharp at any depth. Below 4 px glyph height, text is drawn as a gray bar
  (the tldraw "no text outline below a threshold" idea, but as a simple filled strip), and below 1 px not at all.
- **Grayscale anti-aliasing only**, no LCD subpixel rendering and no hinting, because text is rotated, scaled and
  continuously zoomed.
- **Editing**: caret rectangles, selection rectangles, hit testing from point to index come from Parley's
  cursor/selection APIs and are drawn in the overlay layer. Text entry and IME composition are section 07.

### Level of detail

All thresholds below are in device pixels at the current zoom, to be tuned. There is one size rule for what exists
on the client, set by section 03: an object is sent, and drawn, when its longer bounding-box side covers at least
`eff_min_px` pixels. `eff_min_px` is the `min_px` of the view (default 1) raised by the server when the view's
object or byte budget is hit, and the server reports it in `ViewEnd`. The server's own size test is conservative to
within a factor of two (it may send objects down to half a pixel); this section culls precisely with `eff_min_px`
and never uses a number of its own. Levels play no part: an object that is too small is just below the size
threshold, wherever it sits.

| Screen extent of the object's longest edge | Treatment |
|---|---|
| Over 2^15 px | Cut to the window by section 01, drawn live or via tiles (see below) |
| 8 px and up | Full vector drawing |
| `eff_min_px` to 8 px | Vector drawing in sparse tiles; in dense tiles covered by an overview picture; strokes drawn with their width held to at least 1 px when thinner (a minimum visible width) |
| 0.25 px to `eff_min_px` | Not sent, so not drawn individually; in a picture if the tile uses one; the marker below otherwise |
| Under 0.25 px | Not in any picture; only the index knows it |

- **Stroke simplification.** Below 16 px extent, a stroke is drawn from a reduced outline: points within 0.5 px of
  the line between neighbors at the current scale are skipped, computed at tile-render time from the stored points
  (the stored data is untouched). The simplified path is cached per octave.
- **Dashed and dotted lines** become solid below an extent where a dash is under 3 px (tldraw does the same).
- **Gradients and shadows** are dropped under 4 px.
- **The "more here" marker.** `ViewEnd.more` (section 03) lists, per `PicTile` in view, how many objects were not
  sent (its `pic_count` minus the objects actually sent). A tile with a positive count and no picture shows a small
  mark: a 3 px dot in a neutral gray at the tile's center (the count is the only information section 03 gives), with
  opacity growing with the logarithm of the count. Zooming toward the marker brings the content in. Markers never
  block hit testing, are drawn in the overlay layer (so they never take part in the content order), and are off in
  exported images.
- **Zooming through a level boundary.** The tile key (octave) is global, so there is no jump (see tile cache).
- **No level arithmetic.** Nothing in this section looks at how many levels an object is from the view. The only
  per-object numbers are its pixel extent (from its bounds and the zoom) and its home octave `h(o)` (see Extents and
  the home octave).

#### Large objects seen from far below them

An object much larger than the view (a frame, a backdrop, a filled region) arrives from section 01 already cut to
the padded view in camera-relative `f32` pixel coordinates. Cases:

- **The view is inside a filled shape.** The cut result is a rectangle the size of the padded view (or empty if the
  view is outside). The renderer draws one rectangle with the fill paint. No stroke is drawn, because no edge lies
  inside the padded view. Cost: one rectangle per such object, however large the shape is.
- **An edge crosses the view.** The cut polygon has a boundary of the true edge inside the view and straight sides
  where the cut is on the view margin. The fill is drawn from this polygon; the stroke is drawn only along true
  edges (flagged by the cut) so the cut sides never show a border. Curves are cut into curve pieces that are drawn
  as Béziers, so the edge is sharp at any depth.
- **Camera movement.** Section 01 makes the cut relative to the render origin `R` and keeps it until the camera
  re-bases (the camera moves more than 2V from `R`). This section does not re-cut; it uses the pieces it is given.
  Cuts for large objects are therefore one per object per re-base.
- **Large images.** The same cut gives the visible source rectangle in image space; section 05 supplies the matching
  mip tile; the image is drawn with that sub-rectangle.
- **Order.** A cut object keeps its original order key, so a backdrop from far above still sits behind or in front
  of small items as its order says.
- A cut object is drawn into tiles like any other object. When 01 re-makes a cut after a re-base, the pieces inside
  any tile already rendered are identical, so no tile is marked stale; a tile rendered later uses the new pieces.

### Extents and the home octave

Two pure functions that need no layout and no GPU, defined here once so that section 03 can keep their results per
object in its index as derived data. Nothing here is stored in the document.

**Extent without layout.** Section 02's `ExtentOracle::extent(&self, object: &Object) -> LocalBox` returns the
object's box in its own frame from its fields alone. `LocalBox` is used by 02 but not defined there, so it is
defined here: `pub struct LocalBox { pub min: LPt, pub max: LPt }`, on 01's lattice (2^-40 object units), before the
object's `Xform`. 01's `exact_box` turns it, with the world placement, into an `ExactBox`. All arithmetic uses only
01's allowed operations and rounds outward, so every client and the server get the same bits. By kind:

| Kind | Box |
|---|---|
| Stroke | Bounds of the points on the lattice, grown by the largest half width the brush can give (`base_width`, the pressure table's maximum) times the larger of 1 and the round-join factor, plus one lattice step. Cuts are ignored, so the box covers the whole stroke. Integer arithmetic. |
| Shape | Bounds of `geom` grown by half the stroke width times the miter limit (4) when joins are mitered, else by half the width; arrowheads add their size (a fixed multiple of the width). Dashes add nothing. |
| Image, Video | The box `[0, size.w] x [0, size.h]` from the stored `size`; `crop` does not enlarge it. |
| Group | The union of its children's boxes, each mapped into the group's frame by 01's `compose`. The oracle sees one `Object`, so for a group it returns the empty box and `Doc` forms the union through `compose` from the children's results (section 02 states this). |
| Text | The stored `TextFrame` size (below). The server never lays text out. |

**The text rule.** The box of a text object is the size stored in its `TextFrame`, written by the client that last
edited the text, after layout. That client rewrites it when (a) the text is edited, (b) the width, wrap or padding
changes, (c) the defaults change in a way that alters metrics (font, size, line height), and (d) a font it was
waiting for arrives and the layout changes. Other clients never write it, so two viewers with different results
cannot ping-pong. The box only decides what is sent, cached and invalidated: a client that writes a wrong box harms
only the visibility of that object. To keep a wrong box from leaving stale pixels, the local tile invalidation uses
the union of the stored box and the box of the client's own layout. (Section 02 adds the stored laid-out size to
`TextFrame` for this rule.)

**Home octave.** The *tile octave* `O` is the one in the tile cache: a tile at octave `O` has a scale of `2^O`
pixels per level-0 unit and a side of `2^(9 - O)` level-0 units (512 pixels). For an object with exact box `b`, let
`e(b) = floor(log2(longer side of b))` in level-0 units, computed from the bit length of the exact integer side with
no floating point. Then

```rust
/// Pure function of the box. Extent at octave h is in [4, 8) px.
pub fn home_octave(b: &ExactBox) -> i64 { 2 - e(b) }
/// The octaves whose pictures hold the object: extent at Z is in [0.25, 8) px.
pub fn picture_octaves(b: &ExactBox) -> RangeInclusive<i64> { (home_octave(b) - 4)..=home_octave(b) }
pub struct PicTile { pub octave: i64, pub tx: Cell, pub ty: Cell }       // the RasterKey of the tile
/// The tiles of octave `z` that the box touches: indices are the exact floor of the box edges times 2^(z-9),
/// using 01's `Position::at_level` and `Axis::shr_floor`. At most 2 x 2 for any octave in the range.
pub fn picture_tiles(b: &ExactBox, z: i64) -> impl Iterator<Item = PicTile>;
```

`PictureRef` (named by section 03's `ViewEnd` and `PictureUpdate`) is defined here:

```rust
pub struct PictureRef { pub tile: PicTile, pub blob: BlobId, pub version: u32, pub order_min: OrderKey, pub order_max: OrderKey }
```

At `Z = h` the longer side is in [4, 8) px, at `Z = h - 4` in [0.25, 0.5) px, so every object appears in exactly
five octaves and in at most 20 tiles, however deep it is. `e(b)` needs the exact side as an integer at a known
level; 01's `ExactBox { min, w, h }` provides it, and 01's "API by name" gives the bit length of an `ExactBox` side,
which `e(b)` uses.

### Overview pictures

**What a picture holds.** A picture is a tile of the same 512 x 512 premultiplied RGBA grid as the raster tile
cache, at the same key `RasterKey`, so it is one more source for a cached tile. Content is partitioned by the
object's size, not by the view, so nothing is drawn twice and nothing is dropped between a picture and live vectors:

- Each object `o` has a **home octave** `h(o) = home_octave(box)` (previous subsection): the octave at which its
  longer side is 4 to 8 px. It is a function of the box alone and is kept in the index, not in the document.
- A picture tile of octave `Z` contains the objects with `h(o) >= Z` (at `Z` they are under 8 px) and
  `Z >= h(o) - 4` (at `Z` they are at least 0.25 px): the tiles that `picture_tiles` returns for `Z` in
  `picture_octaves`. Each object appears in five octaves, so building pictures costs at most 20 tiles per object,
  however deep the object is.
- The client draws live all objects with `h(o) < Z_v`, where `Z_v` is the view's tile octave. Objects with
  `h(o) >= Z_v` are drawn from the picture, or live if the tile is sparse.

**Each object is drawn exactly once, given the size threshold.** For a tile at octave `Z_v`, an object is in one of
three classes. (1) `h(o) < Z_v`: its extent at `Z_v` is over 8 px, so on screen over 4 px; it is sent and drawn live
if its extent is at least `eff_min_px` (a server under heavy load that raises `eff_min_px` above 4 px culls it,
which is the threshold doing its job), and it is in no picture of this tile (pictures hold `h(o) >= Z`). (2)
`h(o) >= Z_v` in a dense tile: the server did not send it; it is drawn only through the picture, whose membership
(`h(o) >= Z_v` and extent at least 0.25 px at `Z_v`, that is `Z_v >= h(o) - 4`) does not depend on `eff_min_px`. (3)
`h(o) >= Z_v` in a sparse tile: it is sent if its extent is at least `eff_min_px` and drawn live; if it is smaller
it is neither sent nor in a picture, so it is not drawn, which is the size threshold doing its job. A tile is either
dense or sparse, never both, so for each object and each tile it touches there is exactly one source (live, picture
or neither): no object is in both the picture and the live set of the same tile, and no object that passes the
threshold is in neither. The client applies `eff_min_px` as a cull on the live set too, so an object the server has
not yet withdrawn (`drop` in `ViewEnd`) does not flicker back in.

**When a picture is used.** The server (section 03) knows the count per tile. If the number of objects with
`h(o) >= Z_v` touching a tile is above a threshold (starting value 2,000), it sends the picture reference instead of
those objects, and the client draws the picture. Otherwise it sends the objects and the client rasterizes them. This
is the only difference: a tile always has exactly one source for small objects.

**Order.** A picture is one flat image; objects in it have order keys, and bigger live objects may have keys in
between. The picture records `order_min` and `order_max` of its members. The client draws the picture at
`order_min`. If a live object overlapping the tile has a key within `[order_min, order_max]`, the picture would draw
wrongly over or under it. In that case the server is asked for the members as objects if there are at most 10,000 of
them, otherwise the picture is used as is and the small error is accepted until zoom brings the objects in. Large
backdrops are normally at the bottom of the order, so a conflict is rare. A test counts conflicts on a generated
document. This weakens the high-level design's "one order across all levels" for pictures only.

**Generation.** A picture builder is a headless process that links `everink-core` and `vello_cpu`:

- Input: the objects touching a tile whose home octave satisfies `Z <= h(o) <= Z + 4`, read from the server's store.
- Rendering: `vello_cpu` into a 512 x 512 buffer; it needs no GPU, which a server often lacks, and is the more
  mature Vello renderer. Pictures come from one machine's renderer, so all clients see the same picture even though
  their GPUs differ. They are not recomputed on clients and are never used for hit testing.
- Format: lossless WebP or PNG with alpha, premultiplied on load; compression chosen in the spike by size. Expect 5
  to 60 KB per tile (unverified guess).
- Storage: as blobs in the blob store (section 05), named by the content hash and listed per tile in a table
  `(octave, tx, ty) -> blob, version, order_min, order_max`.

**Invalidation.** A change to an object marks dirty the tiles that `picture_tiles` returns for its old and new box,
in each of its five octaves. The builder debounces: it rebuilds a dirty tile after 5 s without a new change to it,
and at the latest after 60 s of continuous change. Until then clients keep the old picture; the live objects in view
are always current because they are sent as objects when a picture is not used or the view is close. A picture is
stale only for far-away dense regions, which is when exactness matters least. Deleting the last member of a tile
removes the picture.

**Blending to live vectors.** The client treats a picture as the tile's state `Picture`. When it zooms toward a
tile, the tile's octave increases; the picture of the new octave is requested, and meanwhile the previous octave's
picture is shown scaled up (soft). When the objects of that octave have arrived and the sparse-or-dense decision
says "send objects", the client rasterizes the tile from vectors and cross-fades from picture to vectors over 150
ms. A picture octave not available is simply absent: the "more here" marker shows. A tile never shows both picture
and vectors of the same objects, because of the partition above.

**Cost of pictures.** For a surface of one million small strokes, each in 5 octaves, the builder renders about 5
million tile-appearances in the worst case, but a tile holds many objects, so the number of distinct tiles is far
fewer. Cost grows with the number of objects, not the surface area. It is a background job and not on the critical
path of editing. Whether to build them at all for a small room is an operator choice.

### Performance budget per frame

Starting budget for a 60 Hz frame (16.7 ms) on a mid-range laptop in the steady pan case with 10,000 objects in
view. Each line has a test that fails over budget. CPU and GPU run in parallel; the CPU total and GPU total are each
under the frame time.

| Stage | CPU (ms) | GPU (ms) | Note |
|---|---|---|---|
| Input, camera, animation | 0.3 | | |
| Visible-set query | 1.0 | | section 01/core; zero on most frames |
| Placement of new objects (exact step, cuts) | 2.0 | | 01's cut queue budget: 2 ms per frame, the one figure for exact arithmetic; items past it draw from the previous cut |
| Order, dirty-tile detection | 0.5 | | |
| Tile raster (time-boxed) | 4.0 | 3.0 | spread across frames; the budget, not the need |
| Lifted/live objects | 1.0 | 1.0 | selection drag, cut objects |
| Compose (tiles, live ink, overlay) | 0.5 | 1.0 | |
| Controls (section 07) | 0.5 | 0.3 | |
| Present and slack | 1.0 | | |
| Total | about 11 | about 6 | slack for the rest |

The decisive unknown is the rate at which `vello_gpu` rasterizes tiles. If a full-screen re-render of 10,000 objects
(about 20 tiles) takes 60 ms, then after a large zoom change the view is soft for four frames; that is acceptable.
If it takes 400 ms, the tile approach still holds, but zoom feels soft for about a quarter second, and the spike
decides whether to use `vello_cpu` threads, pre-rendered pictures, or lower tile counts. This is the first
measurement to make.

## Failure modes

| Failure | What happens |
|---|---|
| No WebGPU in the browser | WebGL2 backend; if that fails, `vello_cpu` with a banner. Chosen at startup by feature detection, never by user-agent. |
| WebGPU device lost mid-session | Tiles are recreated from vectors, the session moves to WebGL2, and the device class is remembered locally (Figma's experience). Nothing is lost from the document. |
| Texture allocation fails | The budget is halved, tiles evicted, one retry. A second failure shows a "graphics problem" message and moves one backend down. |
| `vello_gpu` panic on a feature not implemented | The permitted-feature rule makes it unreachable by design; a CI test covers each primitive. On native a `catch_unwind` around the tile render drops that tile and logs; on wasm (abort-on-panic) the page reloads into the CPU backend. |
| GPU slow, frames overrun | The tile budget shrinks; compose and live ink keep their rate; softness lasts longer. |
| Font missing or still downloading | Glyphs of that run are drawn in the fallback font from the document's set, or as a placeholder box; the text block is laid out again when the font arrives, and its tiles refreshed. |
| Text layout differs between platforms | Detected by the cross-platform golden test; the fallback plan is stored line breaks (see Text). |
| Stored ink outline differs between platforms | A bug in the deterministic module; caught by the cross-target test, never by users. A `style_ver` allows replacing the algorithm without changing old strokes. |
| Pointer events are coalesced or absent | The modeler gets fewer samples; strokes look slightly coarser; no error. |
| Pen has no pressure | Width from speed, or constant by brush choice. |
| Prediction is wrong | A few pixels of tail show for one frame; never stored. |
| Picture missing, stale or in conflict | The old picture, the marker, or objects are used as described; no blank hole. |
| Huge object with its cut failing (section 01 returns an error) | The object is skipped for that frame and drawn from the last good cut. |
| Hostile content: a path with millions of points, or deeply nested groups | The caps (points per stroke, path nodes, glyphs per text object, picture size) and the per-tile time-out are in section 08's limits table; this section names them and carries no numbers. The renderer skips paths over a cap, and a tile that exceeds the time-out is marked failed and keeps its previous contents. |
| A stroke with many cuts | Past 64 cuts section 02 replaces the stroke by pieces; until then the outline is rebuilt per change and cached with the object version. |

## How it is tested

| Rule | Proved by |
|---|---|
| Stored ink outline is identical on all targets | One corpus of 1,000 random and 100 recorded strokes; outline path bytes hashed on native Linux, macOS, Windows, arm64 and wasm in CI; hashes must match. |
| Outline code uses only allowed operations | The same `clippy.toml` `disallowed-methods` list as section 01 (it is the reference), applied to the outline module, plus a CI check that `relaxed-simd` is not enabled. |
| The outline lies on section 01's lattice | A property test: every coordinate of the built `Outline` is an `i64` produced by the rounding in step 7, and equals the value from an independent big-rational reference to within one lattice step. |
| Incremental and one-shot outlines agree | Random point streams; `OutlineBuilder::push` then `finish` against a one-shot build; byte-identical `Outline`. Section 04's preview test reuses it. |
| Partial erase results are identical on every client | The same corpus; eraser paths applied to strokes with random cut sets; the remaining runs, synthetic end points and outlines compared across targets. |
| Two concurrent partial erases both show | Two clients add different cuts to one stroke; both orders give the same outline, with both gaps. Undo of one gesture restores only its gap. |
| Hit test ignores gaps | A pointer over a gap does not hit the stroke and hits the object beneath; a pointer over a remaining run does. |
| Group move invalidates by region | Move a group of 10,000 members; only tiles touching its old and new world box re-render, and the members are not visited individually. |
| Each object is drawn once under `eff_min_px` | Generated scenes at several `eff_min_px` values (0.5, 1, 2, 6); for every view octave, every object and every tile it touches, count sources: at most one of {live, picture}, and exactly one when its extent is at least `eff_min_px` (live or sparse tile) or at least 0.25 px (dense tile). Counted per object and tile, not per view, so an object spanning two tiles of different density is checked in each. |
| Home octave and picture tiles are pure | Random `ExactBox`es at levels -1000 to 1000: `home_octave` and `picture_tiles` equal an independent big-rational reference; the same box gives the same tiles whatever level it is expressed at; every object has five octaves and at most 20 tiles. |
| `ExtentOracle` bounds the outline | For random objects of every kind, the oracle's `LocalBox` contains the lattice outline, is bit-identical on native and wasm, and for text equals the stored `TextFrame` size. |
| The stroke anchor keeps points in range | Random strokes and the extreme cases (points at `-(2^30 - 1)` and `2^30 - 2`, so the floor in `c` gives `max - c = 2^30 - 1`; points exactly at the 2^30 limit that must end the stroke): every stored point is within `2^30 - 1` and fits `i32`; deltas up to `2^31 - 2` round-trip through the five-byte varint decoded as `i64`; a hand-made stroke with a point past `i32` is refused by the decoder. The pen-down preview and the final stroke are compared point by point in world position for exact equality, including a stroke drawn while the view auto-pans across a camera level change. |
| Live-ink target is only used when given | With `live_ink_target()` returning `None`, the web path draws live ink in the main canvas and nothing panics. |
| Tile keys are stable across camera levels | Zoom from 0.5 to 2047 px per unit and over a camera level change; the set of keys needed is the same function of `O` before and after the change. |
| Layout is the same on every target | Golden file of line breaks and glyph positions (to 1/64 unit) for a multilingual corpus, compared native versus wasm. |
| A static scene costs almost nothing | A benchmark: 10,000 cached objects, no change for 60 frames; CPU time per frame below 0.5 ms and zero tile renders. |
| 60 fps with 10,000 objects at levels 0 and 100 | A benchmark pans and zooms a generated scene on three reference machines; frame time 95th percentile below 16.7 ms; run at both levels. |
| Zooming through a level boundary does not jump | Render the same view just above and just below the boundary; compare pixel images with a small tolerance. |
| No tile seams | Render a large filled shape across tile edges; every pixel on the edges equals its neighbors. |
| A deep view looks right | Draw a shape at level 0, view it at level 100 (and level -100); compare edge positions with a reference built with big-number arithmetic (shared with section 01's test). |
| The view inside a huge filled shape draws one rectangle | Count draw calls for a view inside a shape 10^9 times larger than the view. |
| Picture and vectors never both draw the same object | A scene with objects across the 8 px home-octave boundary; assert each object is in exactly one source for every view octave. |
| Order conflict handling | Generated scene with interleaved backdrop; count and assert conflicts get the described handling. |
| Dirty tiles | Change one object; assert only tiles touching its old and new bounds re-render. |
| Fallback ladder | CI runs the full pixel test suite on each backend (wgpu, WebGL2, CPU) with a per-backend tolerance. |
| Blend space | Draw 50% black over white; the pixel value must equal the sRGB-space result on every backend. |
| Latency instrumentation | An internal counter of event timestamp to submit; a lab procedure with a 240 fps camera on named devices; the result is recorded in the repository, not asserted in CI. |
| Prediction never stored | A property test: the stored points after finish equal those from a run with prediction off. |

## Dependencies

All are licensed in a way compatible with MIT OR Apache-2.0 (permissive, so combining is fine; BSD-3 and MIT notices
are carried in a third-party notices file). Versions are those on crates.io on 2026-11-11.

| Crate | Version | License | Status | Why |
|---|---|---|---|---|
| `vello_gpu` | 0.4.0 | Apache-2.0 OR MIT | Pre-1.0, "slightly less mature than Vello CPU", no API stability; renamed from `vello_hybrid` | Primary renderer, wgpu and WebGL2 |
| `vello_cpu` | 0.4.0 | Apache-2.0 OR MIT | Pre-1.0, "most mature" Vello renderer | Picture builder, CPU fallback, tests |
| `vello` (compute) | 0.12.0 | Apache-2.0 OR MIT | Experimental | Not used |
| `wgpu` | 30.0.1 | MIT OR Apache-2.0 | Active | GPU access, native and WebGPU; must match the version `vello_gpu` builds against (not verified) |
| `kurbo` | 0.13.1 | Apache-2.0 OR MIT | Active (Linebender) | Path and curve types, dashing |
| `peniko` | not checked | Apache-2.0 OR MIT (unverified) | Linebender | Colors, brushes |
| `parley` | 0.12.0 | Apache-2.0 OR MIT | Active, monthly releases | Text layout |
| `harfrust` | 0.16.0 | MIT | Active | Shaping (through Parley) |
| `skrifa` | not checked | MIT OR Apache-2.0 (unverified) | Active (Google Fonts) | Glyph outlines |
| `icu` (ICU4X) | not checked | Unicode-3.0 (unverified) | Active | Segmentation, bidi |
| `glifo` | in development | Apache-2.0 OR MIT (assumed with Vello) | Not released | Glyph cache, color emoji; optional |
| `rstar` | 0.14.0 | MIT OR Apache-2.0 | Active | Spatial index (core, not this section) |
| `lyon`, `tiny-skia`, `cosmic-text`, `femtovg`, `skia-safe` | 1.0.19, 0.12.0, 0.19.0, 0.28.0, 0.157.0 | see text | | Considered; not used |
| Stroke modeler | our own port | Apache-2.0 (derived from the C++ library, credited; NOTICE file) | Ours | Smoothing and prediction |
| `web-sys` | not checked | MIT OR Apache-2.0 (unverified) | Active | Web shell's `Path2D` live-ink canvas |
| WebP/PNG codec | not chosen | | | Picture format, in the builder only; pick a memory-safe crate in the spike |

## What this assumes of other sections

- **01 (positions).** Names used as they appear in 01's "API by name": `Camera` (zoom held in [0.5, 2048) with
  hysteresis), `RenderOrigin` and `Visible` (`Plain` or `Cut`, with true edges flagged, Béziers included because ink
  outlines are quadratic), `Outline`, `Seg`, `LPt`, `ExactBox`, `ExactRect`, `Position`, `Cell`, `Placement`,
  `compose`, `localize`, `relevel`, `Position::at_level` and `Axis::shr_floor`. This section's `RasterKey` is not
  01's `TileKey`. Its float rules are the reference for the outline module, and its `Outline` (`i64` lattice at
  2^-40) is what the stroke builder produces. Also used from 01's table: a tile's origin relative to `RenderOrigin`
  (pipeline step 5) and the bit length of an `ExactBox` side (`home_octave`).
- **02 (document).** Names as in 02's "Types and operations by name": `Xform` (`TopXform`, `ChildXform`),
  `PackedPoints`, `StrokeBrush`, `Cut`, `StrokeParam`, `ShapeGeom`, `StrokeStyle`, `TextFrame`, `Size`, `Rect`,
  `ExtentOracle`, `ObjectEffect`. This section supplies `ExtentOracle` for every kind (see Extents) and defines
  `LocalBox`, which 02 uses without defining. A stroke's points are immutable and its anchor is the center of its
  bounding box. `TextFrame` carries the stored laid-out size (the text rule), and `Doc` forms a group's box as the
  union of its children's boxes through `compose`, because `ExtentOracle::extent` sees one object.
- **03 (sync).** Sends objects crossing the view down to the size threshold; its test is conservative to within a
  factor of two and this section culls precisely with the reported `eff_min_px`. Uses `ViewEnd` (`eff_min_px`,
  `drop`, `pictures`, `more`). May keep `home_octave(box)` and `picture_tiles(box, z)` per object in its index as
  derived data (both are pure functions of `ExactBox`); counts objects per tile and octave; replaces the small ones
  by a picture reference above the threshold of 2,000; stores the picture table and schedules the builder; reports
  `order_min` and `order_max`.
- **04 (presence).** Sends only real (not predicted) ink points. The receiver builds the preview with
  `OutlineBuilder::push`, the same function as the final stroke, into the live layer. The preview is keyed by the
  stroke's `ObjectId`; each frame, if the document holds an object with that id, the renderer skips the preview and
  draws the object in the same frame, and the tile that will hold the object is rendered before the preview is
  dropped so there is no gap (if the tile is not ready the preview stays one more frame).
- **05 (media).** Delivers decoded images with mip levels and a maximum texture size; provides font bytes keyed by
  id for the document font set (Latin only at the first release; no CJK or emoji font); serves picture blobs.
- **07 (shell, controls).** Offers `request_frame()` and creates the web canvases: the main canvas, and the
  `desynchronized` live-ink canvas handed over by `SurfaceHost::live_ink_target()` (this section draws into it and
  creates nothing). The accessibility mirror and hidden text area stay *below* both. Also the pen sample stream
  described in Ink and the text-entry events. `everink-ui` paints through `SceneBuilder` into a `SceneFragment` and
  lays out text through `TextService`, both defined under Scene API, in the same frame; `taffy` layout and AccessKit
  trees are 07's.
- **08 (security).** Its limits table holds the caps on points per stroke (20,000), path nodes, glyphs per text
  block, group depth, cuts and picture size; this section names them and carries no numbers of its own.

## Questions for the owner

Answered on 2026-11-12 and stated in the Design: browsers and the processor-only path, the eraser default,
languages, the first app, the merge model, the controls toolkit and the extra renderer candidate. Still open:

1. **Quality versus speed after zoom.** The tile cache trades a few soft frames after fast zoom for 60 fps. If the
   owner would rather have exact sharpness at some frame cost, say so, and the design moves to option 2.
2. **Overview pictures: who pays.** Building pictures costs server CPU. Should it be on by default for every room,
   or an operator setting? I recommend an operator setting, on by default for rooms above 50,000 objects.
3. **Color.** Display P3 (wide gamut) is left out. Say if designers are a target audience.
4. **Highlighter and blend modes.** Multiply-style highlighters and blend modes are not offered in the first version
   because the renderer lacks some blend modes. Is that acceptable?

## Sources

All pages were fetched or searched on 2026-11-11 unless noted.

- Vello repository README: https://github.com/linebender/vello
- Vello architecture notes: https://raw.githubusercontent.com/linebender/vello/main/ARCHITECTURE.md
- `vello` crate docs: https://docs.rs/vello/latest/vello/ ; versions: https://crates.io/api/v1/crates/vello
- `vello_gpu` docs: https://docs.rs/vello_gpu ; versions: https://crates.io/api/v1/crates/vello_gpu
- `vello_cpu` versions: https://crates.io/api/v1/crates/vello_cpu
- Sparse strips v0.3.0 release notes: https://github.com/linebender/vello/releases/tag/sparse-strips-v0.3.0
- Bunnymark performance issue: https://github.com/linebender/vello/issues/1427
- Raph Levien, RustWeek 2025 slides:
  https://docs.google.com/presentation/d/1f_vKBJMaD68ifBO2j83lBly9Zdk-2bsvj_DIHXxvcuk/htmlpresent
- ETH thesis on sparse strips on the CPU:
  https://ethz.ch/content/dam/ethz/special-interest/infk/inst-pls/plf-dam/documents/StudentProjects/MasterTheses/2025-Laurenz-Thesis.pdf
  (not read in full)
- Linebender in 2026 Q1: https://linebender.org/blog/tmil-25/
- WebGPU support: https://web.dev/blog/webgpu-supported-major-browsers ; summary of other sources via search
- Figma, rendering powered by WebGPU: https://www.figma.com/blog/figma-rendering-powered-by-webgpu/
- tldraw performance and culling: https://tldraw.dev/sdk-features/performance
- Zed, rendering UI at 120 FPS: https://zed.dev/blog/videogame
- Graphite blog index (not conclusive): https://graphite.art/blog/
- Skia CanvasKit: https://skia.org/docs/user/modules/canvaskit/
- ThorVG: https://github.com/thorvg/thorvg
- Chrome, `desynchronized`: https://developer.chrome.com/blog/desynchronized
- MDN, DelegatedInkTrailPresenter: https://developer.mozilla.org/en-US/docs/Web/API/DelegatedInkTrailPresenter
- MDN, getCoalescedEvents: https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents
- getPredictedEvents compatibility (mirror): https://docs.w3cub.com/dom/pointerevent/getpredictedevents
- Android Ink API launch: https://android-developers.googleblog.com/2024/10/introducing-ink-api-jetpack-library.html
- Android stylus latency measurement: https://tbuckley-blog-7b3912.pages.dev/writing/stylus-latency
- Apple Pencil latency (2019): https://www.macrumors.com/2019/06/21/apple-pencil-latency-ipados-developers (from a
  search summary, not opened)
- perfect-freehand: https://github.com/steveruizok/perfect-freehand ; Rust port:
  https://crates.io/api/v1/crates/perfect_freehand
- google/ink-stroke-modeler: https://github.com/google/ink-stroke-modeler ; google/ink:
  https://github.com/google/ink
- Rust port of the modeler: https://docs.rs/ink-stroke-modeler-rs
- Parley: https://raw.githubusercontent.com/linebender/parley/main/README.md ;
  https://crates.io/api/v1/crates/parley
- HarfRust: https://crates.io/api/v1/crates/harfrust ; cosmic-text: https://crates.io/api/v1/crates/cosmic-text
- Other crates: https://crates.io/api/v1/crates/skia-safe , /tiny-skia , /wgpu , /lyon , /femtovg , /kurbo , /rstar

**Claims left unverified**

- Exact Chrome/Firefox/Safari support versions for `getCoalescedEvents` and `getPredictedEvents`; live MDN tables
  not read.
- Whether a WebGPU canvas context honors `desynchronized`; whether delegated ink trails give a measurable gain.
- Web pen-to-ink latency numbers (my 33 to 50 ms estimate has no source).
- Vello's blend color space; `vello_gpu` multithreading and tile throughput on large scenes; its wgpu version match.
- Bit-identical Parley and HarfRust results between native and wasm.
- Glifo's support for COLRv1 color emoji; its release timing.
- Excalidraw culling; MapLibre tile behavior as described; Graphite's current renderer; Rerun and Makepad.
- Licenses and versions of `peniko`, `skrifa`, ICU4X, `web-sys`; the picture size estimates; CanvasKit wasm size.
- WebGPU default maximum texture size of 8192 (taken from the spec as I remember it, not re-read).
