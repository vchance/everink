# Positions, camera and deep zoom

Status: detailed design, 2026-11-11, proposal.

Numbers in this document are derived from stated assumptions or are starting values to test. Where a number is an
estimate and not a measurement it says so. Claims that could not be checked against a primary source are marked
"unverified" and collected at the end of the Sources section.

## Scope

This section covers:

- the **position** type: its exact representation, serialized size and the arithmetic on it;
- the **camera**: what it stores, how pan, zoom and zoom animation work, and why crossing a level or cell boundary
  never shows a jump;
- turning a position into a **camera-relative** number for drawing, with its cost and caching;
- **cutting** curves and shapes that are far larger than the view down to the piece near the view;
- **re-levelling** rules for objects that are scaled, grouped or pasted;
- **hit testing and selection** across levels;
- **bounded surfaces** and the **depth cap**;
- **determinism** between native and WebAssembly;
- **navigation aids** and how a **view rectangle** is expressed for the server.

It leaves to other sections:

| Topic | Section |
|---|---|
| What an object stores, how placement changes merge, undo | 02 document model |
| The spatial index, the wire format of a view subscription, which objects the server sends | 03 sync |
| Cursor and live-ink routing and smoothing | 04 presence |
| Stroke outline generation, brushes, level of detail, GPU scene building | 06 rendering |
| Gestures, shared controls, accessibility of the depth readout | 07 controls |
| Limits per role and tenant, hostile-input policy in general | 08 identity and limits |

## The problem

**One number per axis fails at a measurable depth.** An f64 has 53 bits of precision. On a surface where one screen
pixel is 1 unit, and a point is at distance d from the origin, neighbouring representable numbers are about
d x 2^-52 apart. With d = 1e6 units that is 2e-10 units, fine. After zooming in by 2^40 (a factor of about 1e12)
around a point that was 1e6 units from the origin, one pixel is 2^-40 of a unit (9e-13) and the spacing is still
2e-10: a pixel is smaller than the spacing between representable positions, so the point cannot be placed, and
strokes made there collapse onto a grid. In an f32 (24 bits, 2^-23 relative spacing) the same failure comes after
zooming in by about 2^10 at a distance of 1e3 units. Godot documents the same cliff for games: single-precision step
size grows from about 0.0002 at 2048 units from the origin to about 0.03 above 262144 units
([Godot](https://docs.godotengine.org/en/stable/tutorials/physics/large_world_coordinates.html)).

**Owner decision 1 is unlimited zoom.** There is no depth at which "double is enough". Zooming in by a factor of 2
ten times per second for ten minutes is 6,000 doublings, a zoom of 2^6000 (about 1e1806). The largest f64 is about
2^1024. So the zoom itself cannot be a floating-point number, and a position that must identify a point to within a
pixel at that zoom needs about 6,000 bits.

**Three separate precision needs hide in "position".**

1. *Identity and order:* two positions made on different clients, at very different depths, must compare exactly, so
   the server can decide whether a cursor is inside a view and an index can find the objects that touch a view.
   This needs exact integers of whatever length.
2. *Drawing:* the GPU works in f32 (it has about 7 decimal digits). It must only ever see numbers measured from the
   camera and no larger than a few screens, where f32 is accurate to far below a pixel.
3. *Shape:* the points inside one object (a stroke's samples) are numerous, and cannot each carry thousands of
   bits.

**Concrete size of the cheap option.** If every stroke point were a full-precision coordinate, a 1,000-point stroke
drawn 100 levels deep (about 1,000 doublings per axis) would carry 1,000 x 2 x 1,000 bits = 250 KB. Anchoring the
stroke once and keeping f32 offsets costs about 270 bytes for the anchor plus 8 KB of points.

**The last constraint is time.** Frame time is 16 ms at 60 frames per second. Exact big-integer work for 10,000
visible objects must therefore not run per frame; it runs when something enters the view or the camera has moved far
enough that the cached numbers need re-basing.

## What others do

**Zoomable user interfaces.** Pad++ (Bederson, Hollan, UIST 1994) introduced a zoomable surface with semantic
zooming and with small objects skipped during navigation and refined once the view stops
([Pad++ paper](https://www.cs.umd.edu/projects/hcil/jazz/learn/papers/chi-94-pad.pdf), summary via
[Wikipedia: Zooming user interface](https://en.wikipedia.org/wiki/Zooming_user_interface)). Jazz and its successor
Piccolo are scene-graph toolkits in which nodes carry affine transforms and a *camera* node looks at part of the
graph ([Jazz paper](https://perso.lisn.upsaclay.fr/mbl/Teach/SituatedComputing/2017/papers/ZUI/jazz-uist00-bederson.pdf)).
That is design (c) below: a tree of nested frames. I could not read either PDF through the tools available, so
what these toolkits do at extreme depth is **unverified**; the usual limit of double-precision transforms is the
point of this section, not a documented property of those systems.

**Infinite-zoom drawing apps.** Endless Paper's store listing describes a vector canvas where "interconnected spaces
unfold through infinite zoom" and credits a "64-bit vector rendering engine called Fractile"
([App Store](https://apps.apple.com/app/id1294105620)). The listing does not say how deep it goes or how positions
are stored; its internals are **unverified**. The description (spaces opened by zooming) suggests nested frames, but
that is inference.

**Whiteboards bound their cameras.** None of the common ones aim at unlimited zoom:
- Excalidraw clamps zoom to `MIN_ZOOM = 0.1` and `MAX_ZOOM = 30`
  ([constants.ts](https://raw.githubusercontent.com/excalidraw/excalidraw/master/packages/common/src/constants.ts)).
- tldraw clamps zoom to the first and last of its `zoomSteps`, and can constrain panning to a bounds rectangle with
  behaviors `free`, `fixed`, `inside`, `outside` and `contain`
  ([tldraw camera](https://tldraw.dev/sdk-features/camera)). That vocabulary is a good model for our bounded
  surfaces.
- Figma: a community thread says the canvas spans roughly -65,500 to +65,500 on each axis and that content outside
  behaves unpredictably ([Figma forum](https://forum.figma.com/t/make-the-canvas-infinitive/3159)); this is a user
  report, not documentation, and I could not confirm Figma's zoom limits (**unverified**).

**Games: floating origin and large-world coordinates.**
- Unreal Engine 5 stores world positions in double precision (emulated with a pair of floats in shaders) and, for
  rendering, converts to a *translated (camera-relative) world space as early as possible*, where floats are safe.
  Its earlier "tile + offset" types used a fixed 256k tile and showed visible jitter because precision changed as
  objects crossed tile edges
  ([UE5 LWC](https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-rendering-in-unreal-engine-5)).
- Godot's optional double-precision build gives physics double precision but its shaders stay single precision;
  rendering is made to work by emulation, and 2D rendering gets no benefit
  ([Godot](https://docs.godotengine.org/en/stable/tutorials/physics/large_world_coordinates.html)).
- Kerbal Space Program keeps the active vessel near the origin and moves the world around it (floating origin; its
  Krakensbane system does the related velocity shift)
  ([KSP wiki: Krakensbane](https://wiki.kerbalspaceprogram.com/wiki/API:Krakensbane)); the details are described in
  sources of mixed quality, so treat the exact mechanism as **unverified**.
- Star Citizen's engineers describe moving positioning to 64 bit while GPUs stay 32 bit and "the visible range stays
  within 32-bit limits" ([GamersNexus interview](https://gamersnexus.net/gg/2622-star-citizen-sean-tracy-64bit-engine-tech-edge-blending)).
  Outerra: I found only a 2014 blog remark that 64-bit floats are best avoided for planetary geometry
  ([Outerra blog](https://outerra.blogspot.com/2014)); details **unverified**.
- A blog on floating the origin shows the standard recipe: keep positions in doubles, convert to floats relative to
  the player at render time ([Frozen Fractal](https://frozenfractal.com/blog/2024/4/11/around-the-world-14-floating-the-origin/)).

The lesson shared by all of them: store absolute positions in a wide format, draw in camera-relative narrow numbers,
and convert at a well-defined point. Games stop at doubles (about 1e15:1 range). We cannot.

**Deep-zoom fractal renderers.** They hit exactly this wall. The standard answer is *perturbation theory*: compute
one *reference* point at arbitrary precision, then compute every other pixel as a small difference from it in
ordinary double precision; when a pixel drifts too far, pick a new reference ([Ultra Fractal: perturbation
calculations](https://www.ultrafractal.com/help/formulas/perturbationcalculations.html), [Fractalshades math
notes](https://gbillotey.github.io/Fractalshades-doc/math.html), [Rust Fractal
Core](https://github.com/rust-fractal/rust-fractal-core)). Structurally this is "high-precision anchor plus
low-precision offsets", which is the same shape as our design; the camera is the reference, and re-basing
corresponds to choosing a new reference. I read these secondary sources; Martin's original write-up was not
reachable (**unverified**).

**Map tile addressing.** Slippy-map tiles have zoom 0 to about 18 with 4^n tiles at zoom n
([OSM wiki](https://wiki.openstreetmap.org/wiki/Slippy_map_tilenames)); S2 uses 31 levels (0 to 30), each cell split
into four, in a 64-bit id whose lowest set bit marks the level
([S2 cell hierarchy](https://s2geometry.io/devguide/s2cell_hierarchy)); H3 has 16 resolutions with aperture 7 and
hexagons that do not nest exactly ([H3 table](https://h3geo.org/docs/core-library/restable)). All of them are
*fixed-depth* (the finest cell is about a centimetre in S2) and rely on a 64-bit id. Two things carry over: a
hierarchical cell id gives a prefix property (a parent's id is a prefix of its children's), and square cells that
nest exactly (S2, slippy) beat hexagons (H3) for us because a drawing surface is a plane with axis-aligned views.
What does not carry over is the fixed depth: a 64-bit id has no room for unlimited zoom.

**Robust geometric predicates.** Shewchuk's adaptive-precision predicates (`orient2d`, `incircle`) give exact
answers for floating-point inputs by switching to exact expansions only when a quick estimate is uncertain
([Shewchuk's page](https://www.cs.cmu.edu/~quake/robust.html), not fetched; the Rust port
[`robust` 1.2.0](https://docs.rs/robust/latest/robust/) is MIT OR Apache-2.0). They solve a problem we can avoid
altogether by keeping outline geometry on an integer lattice (see Design, "Outline geometry").

**WebAssembly and floating point.** WebAssembly arithmetic follows IEEE 754 round-to-nearest-even, but: the bits of
a NaN result are nondeterministic (payload chosen among arithmetic NaNs, sign nondeterministic when a NaN is created
from non-NaN inputs), and the relaxed-SIMD instructions "have nondeterministic results"
([WebAssembly nondeterminism](https://github.com/WebAssembly/design/blob/master/Nondeterminism.md),
[spec numerics](https://webassembly.github.io/spec/core/exec/numerics.html)). The relaxed fused multiply-add may round
once or twice depending on hardware
([relaxed SIMD overview](https://github.com/WebAssembly/relaxed-simd/blob/main/proposals/relaxed-simd/Overview.md)).
Rust's float RFC states that `mul_add`, `sqrt` and float casts match IEEE 754-2008 except for NaN bits, and that the
NaN sign and payload are non-deterministic ([RFC 3514](https://rust-lang.github.io/rfcs/3514-float-semantics.html)).
The RFC does not mention `sin` or `powi`, and I found no source for whether native `sin` and wasm `sin` agree
(they may use different libm code), so **assume they do not**.

## Options considered

Four designs were tested against the problem. (a) is the high-level proposal, (b) and (c) were required by the
brief, (d) is what I propose.

- **(a) Level + cell + offset, offsets as f64.** As in `design.md`.
- **(b) Arbitrary precision on every point.** Every coordinate of every point is a big number.
- **(c) A tree of nested frames.** Each frame has a transform (f64) relative to its parent; the camera is a node;
  zooming into a frame descends into it.
- **(d) One global exact plane, per-object anchors, integer offsets (chosen).** Same shape as (a), with three
  changes: offsets are u64 fixed-point, outline geometry is on an integer lattice, and the camera works against a
  re-basing render origin.

Also considered and set aside: **double-double or f128 positions** (106 or 113 bits, about 1e32 of dynamic range:
a cheap interim, but it is a limit, so it violates decision 1) and **keeping only a camera-local float world with
origin shifts** (works for a game, but gives no way to compare two clients' positions exactly).

| Property | (a) level+cell+f64 offset | (b) big number per point | (c) nested frames | (d) chosen |
|---|---|---|---|---|
| Unlimited zoom | yes | yes | yes | yes |
| Bytes per stroke point at depth 100 | 8 (f32 pair) | about 250 | 8 | 8 |
| Anchor cost at depth 100 | about 270 bytes | n/a | one f64 transform per frame | about 270 bytes |
| Exact compare of two positions | yes, but f64 offsets need up to about 1,100 fractional bits (see below) | yes | no: needs a common ancestor | yes, integers only |
| Canonical encoding (one value, one byte string) | no (-0, NaN, denormals) | yes | no | yes |
| Bad input to refuse | NaN, inf, denormal offsets | none | NaN transforms | none for anchors |
| Spatial index over all objects | yes | yes | needs frame-aware index | yes |
| Two clients create content in "the same place" independently | trivially comparable | trivially comparable | two unrelated frames | trivially comparable |
| Re-parent / move across frames | n/a | n/a | recompose transforms, precision loss compounds | n/a (objects are never in a frame) |
| Per-frame cost | bigint subtract per visible object, unless cached | bigint work per point | compose a chain of transforms per object | cached; bigint work only on re-base |
| Determinism native vs wasm | needs care (floats everywhere) | integers | needs care | integers for identity; floats only in a bounded, listed set |
| Cost of an extra level of zoom | about 20 bits per position | about 20 bits per point | one frame | about 20 bits per position |

**Why (a) as written is not enough.** The high-level design says the offsets are "exact binary fractions" and
compared exactly. That is true of every f64, but the *range* of an f64 binary fraction is about 2^-1074 to 2^1023:
an offset such as 1e-300 is an exact dyadic number that needs about 1,000 fractional bits. A "bounded" exact
comparison does not exist for f64 offsets; it needs bit widths set by the data. An attacker can send offsets with
exponents all over that range to force wide arithmetic. Offsets also have several encodings of equal values (-0 and
0), non-finite values to refuse, and non-uniform spacing (spacing near 1,000 is 2^-43, near 1e-3 it is 2^-62),
which means moving an object by "one step" is not the same distance everywhere in the cell. A u64 fixed-point
offset has none of these problems and gives up nothing we need (see Design: resolution is 2^-54 of a level unit,
2^44 steps per pixel at the largest in-level zoom).

**Why not (b).** It is the simplest model and it is correct. It loses on size (a point costs the depth, and a
stroke has thousands of points) and on time (every point of every visible object would pass through big-number
subtraction each time the camera moved). Anchoring once per object keeps the big-number work per object, not per
point.

**Why not (c), honestly.** Nested frames are numerically the easiest design: every number is a small f64, nothing
needs big integers, and Pad++/Jazz-style toolkits and apparently Endless Paper work this way. It fails our other
requirements, not the numeric one:
1. *Concurrent users.* If two people zoom into "the same" spot and each draws, they must both end up in the same
   frame or in frames that can be compared. A tree needs a rule for creating frames that every client agrees on;
   a global plane needs none, because a position names a place.
2. *The server index and presence.* "Is this cursor inside that view" and "which objects touch this view" need a
   common coordinate system across all objects. With frames the server walks and composes transforms to answer
   each query, and rounding errors compound along the chain.
3. *Moving an object between frames* (dragging a stroke out of a region into its neighbour) means re-expressing it
   through a chain of transforms, and the error compounds.
4. *Order across scales.* The high-level design wants one order sequence across all levels; with frames, sibling
   frames overlap without a defined relation.
What is given up by choosing (d) over (c): the free pass on big-integer code, and the possibility of a camera whose
cost is independent of depth. (d)'s per-frame cost is independent of depth thanks to caching, but the first time
something enters the view the subtraction cost is proportional to its depth.

**Why (d).** It is (a) with its undefined parts defined: every position is an integer, so equality, ordering,
hashing, serialization and cross-level conversion are exact and canonical; everything drawn is camera-relative.
The perturbation-theory shape (a high-precision anchor, low-precision deltas) is the same one fractal renderers
adopted for the same reason.

## Design

### Positions

A **level** is an integer `l`. One **unit** of level `l` is 1024^-l of a level-0 unit (level 1 is 1024 times
finer). A position on one axis is a **fixed-point integer** `n` in steps of 2^-54 of a level-`l` unit:

```
value (in level-l units)  =  n / 2^54
n = cell * 2^64 + frac          cell: signed integer of any length;  frac: u64
cell covers 1024 level-l units  (2^64 steps = 2^10 units), and equals exactly one unit of level l-1
```

The split into cell and offset is therefore just the high and low part of one integer; `cell = n >> 64` (floor) and
`frac = n mod 2^64`. The offset is `frac / 2^54` units, in [0, 1024). The hierarchy of the high-level design is
preserved (a level-`l` cell is one unit of level `l-1`, so each cell has one parent cell and 1024 x 1024 children).

```rust
pub const LEVEL_SHIFT: u32 = 10;   // 1024 = 2^10 between levels
pub const FRAC_BITS: u32 = 64;     // bits of offset inside a cell
pub const UNIT_BITS: u32 = 54;     // steps per level unit = 2^54  (FRAC_BITS - LEVEL_SHIFT)

pub struct Level(pub i32);         // |level| <= DepthCap::max_level, default 1024

/// Signed integer, canonical form: `Small` whenever the value fits in an i64.
pub enum Cell { Small(i64), Big(BigCell) }       // BigCell wraps the chosen bigint crate; never stores a value
                                                 // that fits Small, never has leading zero limbs
pub struct Axis { pub cell: Cell, pub frac: u64 }
pub struct Position { pub level: Level, pub x: Axis, pub y: Axis }
```

`Axis` with a `Small` cell is exactly an `i128` worth of information, so all common-case arithmetic runs in 128-bit
integers with no allocation (see "Cost").

**Operations (all exact).**

```rust
impl Position {
    /// Express at a finer-or-equal level: shift each `n` left by 10 * (to - self.level) bits. Never rounds.
    pub fn at_level(&self, to: Level) -> Position;          // requires to >= self.level
    /// Express at a coarser level, rounding toward minus infinity (floor). Loses 10*(from-to) bits. Only the
    /// camera and re-levelling use it; comparisons never do.
    pub fn at_coarser_floor(&self, to: Level) -> Position;
    pub fn cmp_axis(&self, other: &Position, axis: Axis2) -> Ordering;   // converts the coarser to the finer
    pub fn sub(&self, other: &Position) -> Delta;           // exact difference at the finer level
}
pub struct Delta { pub level: Level, pub dx: BigInt, pub dy: BigInt }   // steps of 2^-54 level units
impl Delta {
    /// Round to nearest even to 53 bits, then scale by an exact power of two into units of `unit_level`.
    pub fn to_f64(&self, unit_level: Level) -> [Rel; 2];
}
pub enum Rel { Near(f64), Far(Sign) }   // Far when |value| >= 2^1000 units; such an object is never in a view
```

`Delta::to_f64` is implemented by our code on top of the big integer's bit length and top 53 bits plus a sticky bit,
not by the crate's own float conversion, so that the rounding rule is ours and identical on every target. Values
below 2^-1000 units become 0.

**Resolution.** A step is 2^-54 level units. At the largest in-level camera zoom (see Camera: 2048 pixels per unit)
one pixel is 2^-11 units = 2^43 steps. Anchors are therefore 8.8e12 times finer than a pixel at the zoom they were
made at. Sub-anchor detail is carried by the object's own geometry (next section), so the anchor lattice never
limits visible precision.

**Canonical serialization.** One value has one byte string, so equality, hashing and signing can use the bytes.

```
position := zigzag_varint(level)  axis(x)  axis(y)
axis     := varint(len * 2 + sign)  magnitude[len]  frac[8]      // magnitude big-endian, no leading zero byte
                                                                 // cell 0 is len = 0 and sign = 0
```

A decoder rejects: a leading zero byte, `sign = 1` with `len = 0` (negative zero), a level or length above the cap,
and trailing bytes. Sizes:

| Position | Bytes |
|---|---|
| Level 0, cell 0 | 1 + 9 + 9 = 19 |
| Level 0, cells below 256 | 1 + 10 + 10 = 21 |
| Level 100, near the origin (cell about 1,000 bits = 125 bytes per axis) | 2 + 2 x (2 + 125 + 8) = 272 |
| Depth cap, default (cell 12,288 bits = 1,536 bytes per axis) | 2 + 2 x (2 + 1,536 + 8) = 3,094 |

That is about 20 bits (2.5 bytes) more per position for each level deeper: 10 bits per axis. This confirms the
high-level design's "about twenty more bits". At depth 100 a surface with 100,000 objects spends about 27 MB on
anchors if each is stored whole, which is too much; section 03 and 02 should encode anchors **relative to a base
position in the same message or chunk** (the difference of two nearby anchors has few significant bytes). Section 03
may also derive an ordered index key by interleaving the bits of x and y (a Morton key): it has the S2 prefix
property, a parent cell's key is a prefix of its children's.

### Why 1024

The ratio is a convention with consequences; this is what each constraint says.

1. *It must be a power of two*, so changing level is a bit shift of an integer and a multiply by an exact power of
   two of a float. A non-power-of-two (say 1000) would make every cross-level conversion a big multiplication and
   would round floats.
2. *The in-level zoom range must cover what one view needs.* In a view, an object spans from about one pixel to
   several screens (about 20,000 pixels if you pan while drawing): a ratio of about 2^14.3. The camera holds zoom in
   [0.5, 2048) pixels per unit (next section), so object sizes in units fall in [2^-12, 2^16): a width of 28 binary
   orders. An object's own coordinates are f32-or-lattice, which is scale-free; the band only has to fit the integer
   lattice (below) and keep sizes well away from the ends of that lattice.
3. *Bits per doubling do not depend on the ratio.* Position cost is 1 bit per axis per doubling of zoom, whatever
   the ratio. So 1024 does not save space; 16 or 2 would not cost more.
4. *Smaller ratios mean more re-levelling.* With ratio 2, an object scaled by 3 changes level. With 1024 an object
   keeps its level over a 2^28 range of sizes, so levels change only after large rescaling.
5. *Larger ratios widen the range that lattice and f64 conversions must cover.* With 2^16 the band is 2^44 wide,
   which does not fit a 2^-40 lattice in an i64 with margin.
6. *It matches `design.md`* (a thousandth to a couple of thousand units), so the index of section 03 can still use
   cells and levels as named.

So 1024 is a sound middle, not a unique answer. Any power of two from 2^6 to 2^12 would work with minor changes to
the constants below; 2^10 is kept.

### Camera

```rust
pub struct Camera {
    pub level: Level,            // camera's own level; has no meaning outside the camera (see "View rectangle for the server")
    pub center: Position,        // exact; on `level`
    pub zoom: f64,               // logical pixels per unit of `level`, kept in [0.5, 2048)
    pub viewport: Size,          // logical pixels, from the shell
    pub limits: CameraLimits,    // from surface limits; may be unbounded
}
```

The camera is **local state, not shared**. Views travel only as sync messages (`SetView`, section 03), never in
presence; nothing sends the zoom as a float. So float drift in `zoom` is harmless; the position `center` is exact.

**Why [0.5, 2048) and not [1, 1024).** The renormalization has **hysteresis** so that a camera hovering at a
boundary does not change level every frame (each change invalidates cached numbers):

```
normalize():
    while zoom >= 2048:  level += 1;  center = center.at_level(level);  zoom /= 1024     // exact
    while zoom <  0.5:   level -= 1;  center = center.at_coarser_floor(level);  zoom *= 1024  // exact in float
```

After going finer, zoom is at least 2; after going coarser, at most 512; going back takes a factor-of-4 change.
`/= 1024` and `*= 1024` are exact (powers of two) and `at_level` is a shift, so **finer is lossless**. Coarser drops
10 bits of the center below 2^-54 coarse units; at zoom < 512 pixels per coarse unit that is under 2^-45 pixel. A
zoom-out then zoom-in round trip can drift by that much; over a million round trips that is below 3e-8 pixel.

**No visible jump.** The view of the world depends only on `(center, zoom * 1024^level)` and `normalize` preserves
both exactly (finer) or to 2^-45 pixel (coarser). Cell boundaries are not special at all: `center` is one integer.
A test projects fixed points before and after (see "How it is tested").

**Operations.** Each computes small f64 deltas and adds them to the exact center.

```rust
impl Camera {
    pub fn pan_by_px(&mut self, dx: f64, dy: f64);                 // center += round(dx / zoom * 2^54) steps
    pub fn zoom_about(&mut self, focus_px: [f64; 2], factor: f64); // focus point stays under the cursor
    pub fn zoom_to_fit(&mut self, rect: &Bounds, pad_px: f64);
    pub fn set(&mut self, center: Position, z: Scale);             // exact jump, used by "go to"
    pub fn clamp(&mut self);                                       // CameraLimits: zoom range, pan bounds
}
```

`zoom_about` for a focus offset `d` (units, from the screen center, f64) and old and new zoom `z0`, `z1`: the new
center is `center + d x (1 - z0/z1)`. `d` is at most half a screen in units (at most about 4,000 units at zoom 0.5),
so the f64 result has absolute error under 1e-12 unit, 2e-9 pixel at the largest zoom. It is converted to steps
(`round(delta * 2^54)` into an `i128`, then added). No big-number multiplication is needed.

**Zoom animation.**
- *Wheel and pinch:* multiply zoom by a factor per event, with the focus point fixed. Per frame the factor is
  limited to 2^10, and a loop applies `normalize` after each partial step if more is needed.
- *Zoom buttons, double click, keyboard:* ease the **logarithm** of the total scale over 150 to 250 ms (ease in
  log-space gives constant perceived speed), keeping the focus fixed.
- *Jump to a saved place, a person, or home:* the path of van Wijk and Nuij, "Smooth and efficient zooming and
  panning" ([InfoVis 2003](https://researchr.org/publication/WijkN03/related); abstract read, the full paper was
  not). It zooms out until both ends are visible, pans, and zooms in. The path is a function of the pan distance `D`
  between the two centers, the two view widths, and a trade-off parameter (they suggest 1.42). We compute `D`
  exactly as a `Delta` and use only its logarithm in f64 (bit length plus top bits). Duration is `path_length /
  speed`, clamped to [0.4 s, 3 s]. For a jump of thousands of doublings the apparent speed is very high; that is
  acceptable and similar to fractal zoom films.
- *Precision of the interpolated center.* In the first half of the path the center is `A + f x D`, in the second
  half `B - (1-f) x D`; `f x D` is a bignum times an f64 (take the top 64 bits of `D`, multiply, shift) and has
  relative error 2^-53 of the distance already travelled. Computing the second half from `B` is what makes the
  **end** exact: the last frame places the center exactly at `B`. The error at the middle is 2^-53 of `D`, and the
  view there is about as wide as `D`. This property (the error stays below a thousandth of a pixel over the whole
  path) follows from the path shape but was not proven here; it is tested.
- Animations are interrupted by any user input and restart from the current state.

**Gestures that need rotation** (canvas rotation) are not in scope. The camera has no rotation; adding one later
only changes the view rectangle to its axis-aligned bounding box.

### Turning a position into a camera-relative number

Three quantities are separated: the exact camera `center`, an exact **render origin** `R`, and cached f64 values
relative to `R`.

```rust
pub struct RenderOrigin {
    pub at: Position,            // exact; on the camera's level at the time it was set
    pub half_window: f64,        // W': half-side of the cache window in camera-level units
    pub generation: u64,
}
pub struct Visible {             // one entry per drawn object, rebuilt on re-base
    pub id: ObjectId,
    pub kind: Prepared,          // Plain { origin_rel: [f64;2], path: PathRef }  |  Cut { segs: Vec<CutSeg> }
}
```

Let `V` be the viewport in camera-level units (width in pixels divided by zoom; the larger side). Then:

1. **Re-base trigger.** When `|center - R|_inf > 2V` (checked as an f64 from one exact subtraction per frame), set
   `R = center` and bump `generation`. Also at start and after any `set`.
2. **Window.** Objects are prepared if their exact bounds touch `W'`, the square of half-side `4V` around `R`.
   Everything is prepared from the exact anchor, not from earlier cached values, so re-basing does not accumulate
   rounding error.
3. **Plain object** (its whole bounding box lies inside `W''`, the square of half-side `16V`): store `origin_rel =
   (anchor - R)` converted to f64 in camera-level units (`Delta::to_f64`), and the object's path in its own local
   f32 coordinates. Nothing about the path is recomputed.
4. **Cut object** (anything larger): run the cut below and store the pieces already relative to `R`.
5. **Per frame, per object** (all f64, then f32 for the GPU): `t = (origin_rel - (center - R)) * zoom`. `center - R`
   is one exact big subtraction per frame, converted once. The per-object transform is `translate(t) * scale(zoom *
   2^(10*(camera.level - object.level)))`; a few flops per object. Pan changes only these translations; the path
   data stays uploaded.
6. **Level change of the camera.** `R`, `origin_rel` and `half_window` are all in camera-level units, so a camera
   level change multiplies them by 1024 or 1/1024: exact, no recomputation, no big-number work.

**Why this is accurate.** A cached `origin_rel` has magnitude at most `4V` (plain objects at most `16V`); an f64 has
2^-53 relative error, so the absolute error is at most `16V x 2^-53 = 1.8e-15 V`. A screen of 4,096 pixels gives
`7e-12` pixel. The per-frame subtraction adds another rounding of the same size. The values handed to the GPU are
at most about `18V` = 74,000 pixels from the screen center; an f32 near 74,000 has spacing 2^-7 = 0.008 pixel,
which is the worst case, for geometry at the edge of the window and well off screen. Near the screen center (the
visible area, within `V/2`) the spacing is under 0.001 pixel.

**Cost (estimates, not measurements).**
- Exact subtraction of two anchors: the common case (both cells `Small`, levels within 3 of each other) is
  `i128` arithmetic, about 2 ns. A deep case is one big subtraction on `ceil((64 + 10 d) / 64)` words, where `d` is
  the depth in levels: at depth 100 that is 17 words, roughly 20 to 40 ns with a careful in-place implementation.
- Re-base for 10,000 objects: 10,000 subtractions, 10,000 conversions: about 0.5 ms at depth 100, 0.05 ms at
  shallow depth. At one screen of panning per second and a re-base every 2V this runs about once per two
  seconds.
- Per frame: one big subtraction (`center - R`) and 10,000 x about 10 flops: about 0.05 ms. This meets the
  "exact arithmetic under a millisecond per frame at level 100" target of `design.md` with a wide margin, because
  the exact step runs on re-base and on object entry, not every frame. **Changed from `design.md`:** it said the
  exact step runs "when the camera crosses into another cell"; the trigger here is distance from the render origin,
  because a cell is 1024 units, which is anywhere between half a screen and a million screens depending on zoom.

Entry into the window happens when section 03 delivers an object, when a rebase brings an already-held object into
the window, or when a pan ends. These are the only events that call `Delta::to_f64` for an object.

### Exact comparison: when it runs

| Event | Comparison | Where |
|---|---|---|
| Object enters the cache window | bounds vs `W'` (exact) | client |
| Re-base | anchor - R for every held object | client |
| Which objects does a view need | view rectangle vs index cells and object bounds | server (03) |
| Is a cursor inside a viewer's view (requirement 7) | position vs the view's `ExactRect` (grown by 1/8 in 03), both axes | server (04) |
| Marquee fully contains an object | object bounds vs marquee corners | client |
| Server checks a bounded surface | object bounds vs `Bounds`, level vs `ZoomRange` | server (08/02) |
| Placement is accepted | bit lengths vs `DepthCap` | server |

All use `cmp_axis` which converts the coarser operand to the finer level with a shift, without allocating (it
compares sign, then bit length adjusted by the shift, then limbs). It costs one comparison of up to `d`-word
numbers. It is never used for z-order (order keys do that).

### Outline geometry and determinism

**Contract with sections 02 and 06.** A **world placement** is where an object is on the surface, after its
ancestors have been applied (see "Groups: compose, localize, relevel"):

```rust
pub struct Placement {
    pub level: Level,
    pub anchor: [Axis; 2],        // exact
    pub xf: Linear,               // [a, b, c, d] as f64, column-major: x' = a x + c y, y' = b x + d y
}
```

What section 02 stores is an `Xform` (a top-level `Top { anchor, linear }` or a `Child`, one register), and the
world placement is computed from it. Stored stroke points are section 06's `i32` lattice at 2^-16 of an object unit,
relative to the stroke's anchor, plus a pressure byte; they are immutable. A stroke's anchor is the center of its
bounding box (section 06 owns this rule), so `i32` points at 2^-16 of a unit, which reach 2^15 units, always cover a
stroke inside the 2^16 size band. Section 06's outline builder reads those points, applies the world placement's
`xf`, and rounds the result onto the `i64` lattice below. The renderer derives an **outline** in the object's own
frame:

```rust
pub struct LPt { pub x: i64, pub y: i64 }                 // units of 2^-40 of an object (level) unit
pub enum Seg { Line(LPt, LPt), Quad(LPt, LPt, LPt), Cubic(LPt, LPt, LPt, LPt) }
pub struct Outline { pub segs: Vec<Seg>, pub closed: bool, pub fill: bool }   // |coordinate| < 2^61
```

Everything downstream of the outline (bounds, cutting, hit testing, containment) works on these integers. Therefore:

- **Bounds** are the exact bounding box of the lattice points, converted to world position with a shift and add. Two
  machines derive the same bounds iff they derive the same lattice points.
- **Orientation tests** (which side of a segment a point lies on) are exact in `i128`: with coordinates below 2^61
  the differences are below 2^62, products below 2^124, and the cross product below 2^125. No adaptive predicates
  are needed. This is the reason not to depend on Shewchuk's predicates for outlines; the `robust` crate stays
  optional for float-only geometry elsewhere.
- **Range.** 2^61 lattice steps = 2^21 object units, far above the 2^16 relevel threshold.

**Producing lattice points from floats** is the only place floats feed shared geometry. The rules below are the
**reference list**: section 06's outline builder must use exactly this set of operations and no others, and its
rounding onto the `i64` lattice is the one stated at the end of the list. A lint enforces the list (clippy
`disallowed_methods` plus a CI grep, listed in "How it is tested"):

Allowed in `geometry`: `+ - * /` on f32 and f64, `sqrt`, `abs`, `neg`, `copysign`, comparisons, `floor`, `ceil`,
`trunc`, `round_ties_even`, integer conversion with `as` (saturating, NaN becomes 0, in Rust's defined semantics),
`f32 <-> f64` conversion, and bit operations on floats.

Not allowed: `mul_add` (explicitly fused: correct in IEEE terms, but wasm has no fused instruction outside relaxed
SIMD, so it would fall back to a slow software routine on some targets; the point is to keep the set of operations
small), `sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `powf`, `powi`, `cbrt`, `hypot` (platform libm may differ native
vs wasm), `min`/`max`/`clamp` on floats that may be zero (the sign of a zero result is not specified in the same way
across targets; use `if a < b`), any relaxed-SIMD instruction, and `-ffast-math` style flags (Rust has none by
default). Auto-vectorization must not use relaxed SIMD: the build never enables the `relaxed-simd` target feature in
`everink-core`.

- *FMA contraction.* Rust does not fuse `a * b + c` on its own as far as the RFC implies (it says strict IEEE
  semantics preclude such transformations, without stating it as a rule); I could not find a statement that a Rust
  compiler never contracts. The CI test of equal output on x86-64 (with FMA hardware), aarch64 (FMA native) and
  wasm32 catches a change. Marked **unverified** as a guarantee.
- *NaN and infinity.* Nothing that is NaN or infinite may enter. Every float entering from outside (file, wire, tool
  input) is checked `is_finite()` at the boundary and refused otherwise. Inside `geometry`, inputs are finite and
  bounded (|x| < 2^40, and either 0 or |x| >= 2^-60) so no operation can produce NaN or infinity. The last step
  checks `is_finite()` and the |x| < 2^61 lattice range and refuses the object if not. Since refused objects never
  reach shared state, wasm's NaN-bit nondeterminism cannot appear in anything compared across machines.
- *Subnormals.* The 2^-60 lower bound keeps everything far from the subnormal range, so flush-to-zero modes (some
  32-bit ARM NEON configurations; **unverified**) cannot matter.
- *Rotation and scale are stored as the matrix numbers* `[a, b, c, d]`, not as an angle. A rotate tool computes
  `cos` and `sin` once with whatever library it likes, stores the resulting numbers in the op, and every later
  derivation uses those stored numbers. Rotation by a stored matrix is two multiplies and an add per coordinate.
  Normalizing a matrix uses `sqrt`, which is correctly rounded.
- *Libm.* `libm` 0.2.16 (MIT) is not needed by the shared geometry. Camera animation (local state) may use `std`
  functions (`exp`, `tanh`), which can differ between native and wasm: that is acceptable because the camera is not
  shared.
- *Rounding to the lattice* is `(v * 2^40).round_ties_even() as i64`, with `v` in object units, after the range
  check.

The determinism claim of `design.md` is **narrowed** here. It said a shape's edge falls "at the same pixel on every
client". Pixel output depends on GPU rasterization and cannot be promised identical. What is promised, and tested,
is that the **lattice outline and its exact bounds are bit-identical** on every target. That is what the server's
index, cut and hit testing rely on.

### Cutting large shapes down to the piece near the view

A shape from many levels above the view (a backdrop whose edge you are now zooming into) has an outline whose points
are in object units; the window `W'` is tiny in them. In the case of level difference `Δ` (the view is `Δ` levels
below the object) the window is about 2^(-10 Δ) of the object. The cut produces, from the lattice outline, a short
list of segments in camera-level units relative to `R`, whose rendering inside `W` is identical to the full shape's
(to within the tolerance below), and whose coordinates are safe for f32.

**Step 0: move the window into the object's lattice.** Compute `(R - anchor)` exactly as a `Delta`, scaled to the
object's lattice (shift by 40 bits plus the level difference) giving `Wc` and the window side `w` as integers (big
if `Δ` is large). If the window does not reach the outline's bounding box and does not lie inside the shape (see
Step 3), the object is skipped.

**Step 1: classify by bounding box.** Take a segment and its control-point bounding box (lattice integers, exact).
- If the box does not meet the expanded window `W'` (side 8V around R): replace the whole segment by its **chord**
  (start to end point). *Rationale:* the region between a curve and its chord lies inside the convex hull of its
  control points, which lies inside the box, which does not meet `W'`. Points inside the window therefore have the
  same winding number and the same coverage with the chord as with the curve.
- Otherwise bisect the segment at `t = 1/2` with de Casteljau in integer arithmetic (each midpoint is a sum of
  control points divided by a power of two) and recurse on both halves.

**Step 2: limit the arithmetic.** Bisection adds up to 3 fractional bits per level of depth. To keep the numbers
bounded, after subtracting `Wc` every coordinate is held as an integer with the lattice set to `2^-40 w` (a
fixed fraction of the window side), rounding toward minus infinity at each step. The rounding error of any
coordinate is at most `2^-40 w`; each of the pieces that survive is within `2^-38 w` of the true curve; with
`w` of 8 screens of 4,096 pixels that is under 4e-8 pixel. Where the segment's box meets `W'`, recursion stops when
either:
- the piece's control points lie within `2^-24 w` of its chord (it is a straight line for our purposes; the
  true curve deviates by less than 0.002 pixel for a window of 30,000 pixels), and it is emitted as a **line**,
  clipped to `W''` (Liang-Barsky); or
- the box side is below `4w`, and it is emitted as a **curve** with control points converted to f64 relative to `R`.

The flatness shortcut halves the depth: a smooth piece of size `s` deviates from its chord by about `s^2 / S`
(for curve size `S`), so the recursion stops at `s ~ sqrt(S w 2^-24)` instead of `s ~ w`, which is about
`5 Δ + 12` bisections instead of `10 Δ`.

**Step 3: fills.** Pieces replaced by chords keep the outline closed, so a fill's inside is still correct inside the
window. If every piece ends outside `W'`, the chord polygon is clipped to `W''` with Sutherland-Hodgman, which
returns either the whole rectangle (window inside the shape) or nothing. The same result answers "is the camera
inside this shape".

**Cost (estimate).** The number of bisection nodes is about 2 per depth step. The width of numbers is about `10 d`
bits at the start and shrinks. For `Δ = 3` everything is `i128`: tens of nanoseconds per segment. For `Δ = 100`
(1,000 bisections, average 8 words, 8 coordinates each): about 64,000 word operations, 30 to 60 microseconds per
curve. It grows as `Δ^2`: at `Δ = 2,000` (the cap's worst case) about 15 milliseconds per curve. Results are cached
by `(object, generation)` and recomputed only on re-base or when the object changes. A faster algorithm exists
(find the parameter of the window center by Newton iteration in big integers, then re-expand the cubic around that
parameter, the same idea as perturbation theory); it is **not** specified because its cost is not needed yet and
its root-isolation cases (several crossings, cusps) are harder to make robust.

**Budget.** The cut runs in a work queue with a budget of 2 ms per frame. An object whose cut is not finished is
drawn from its previous cut if one exists, and otherwise not drawn and marked "pending" (the shell may show a
placeholder). This keeps a hostile document (1,000 enormous curves that all cross the window) from freezing the
client; see Failure modes.

### Re-levelling

An object's **size** is the larger side of its exact bounding box in its own units, `s`. Size is read from the
lattice outline, so it is deterministic.

| Event | Rule |
|---|---|
| Created by drawing | Level = camera level. Size lies in [2^-12, 2^16) by construction (see "Why 1024"), so no re-level. |
| Resized, scaled, or group-scaled | After the operation, if `s` is outside **[2^-12, 2^16)**, re-level. |
| Pasted, library template placed, dropped | Always compute `s` and re-level if outside the band (below). |
| Moved without scaling | Never re-levels; only the anchor changes. |

When it fires, choose the level change `k` (an integer, positive means finer) so the size lands in [1, 1024):
`s < 2^-12`: `k = ceil(-log2(s) / 10)`; `s >= 2^16`: `k = -floor(log2(s) / 10)`. The hysteresis (re-level at
2^-12 or 2^16, aim for 1 to 1024) means an object is not re-levelled again by a small scale change.

**What re-levelling changes** for a top-level object (one register write, `id` unchanged):

- `level += k`.
- `anchor`: when `k > 0` (finer), shift left by `10 k` bits (exact). When `k < 0` (coarser), floor to the coarser
  lattice. The dropped fraction (under 2^-54 of a *new* unit, which is under 2^-43 pixel at the largest in-level
  zoom) is not recorded: a placement has no translation other than the anchor, and no pixel can show it.
- `linear` is multiplied by `2^(10 k)` (exact in f64; entries stay finite because `|k|` is bounded by the cap).
- The stored stroke points do not change at all. The outline is derived again.

So re-levelling **finer** is exact, and re-levelling **coarser** moves nothing by more than 2^-54 of the new unit.
Undo stores the previous `Xform` whole. The exact function is in "Groups: compose, localize, relevel" below.

**Pasting.** The clipboard carries the objects, each with its placement relative to the copied selection's center
(an exact `Delta` in the selection's level) and the **screen size** the selection had when copied, as
`(width_px)`. On paste into a view, the objects are scaled so their selection has the same screen width, then
centered on the view or at the pointer; re-levelling runs per top-level object. This is a choice of default
("apparent size preserved") and is listed under the owner's questions. The alternative, preserving world size,
makes a paste from a deep view into a shallow one invisible.

### Groups: compose, localize, relevel

Section 02 owns the group model, and this section follows it: a child's placement is **relative to its parent**, a
group stores no list of children, and moving, rotating or scaling a group is **one write to the group**. A child is
at most `MAX_LEVEL_SPAN = 8` levels from its group (80 binary orders of scale). Nothing here rewrites a member.
The server index and the cut work on the **world placement** that `compose` computes, never on the stored forms.

**What a child stores.** Section 02's sketch has `Child { offset: [f64; 2], linear }`. That cannot support a span
of 8 levels, so this section needs a different offset type (a mismatch to resolve with 02; the reason follows).
An f64 offset is accurate to 2^-53 of its own size. A child that is `d` levels finer than its group can be
2^(-10 d) of the group's size, and the offset reaches the group's size, so the child's position would be wrong
by `2^(10 d - 53)` child sizes: an eighth of its size for `d = 5`, 128 sizes for `d = 6`, and 2^27 sizes for
`d = 8`. With a span of 8 an f64 offset is useless. The type needed is exact:

```rust
pub const MAX_LEVEL_SPAN: i32 = 8;
pub struct Linear { pub m: [f64; 4] }              // [a, b, c, d]; x' = a x + c y, y' = b x + d y
/// Offset of the child's origin from the parent's origin, in the PARENT's frame (before the parent's `linear`),
/// as a signed integer number of 2^-54 units of the CHILD's level. |value| < 2^160, so at most three limbs.
pub struct RelOffset { pub x: Fixed, pub y: Fixed }
pub struct ChildXform { pub dl: i8 /* -8..=8: child level = parent level + dl */, pub off: RelOffset, pub linear: Linear }
pub struct TopXform   { pub anchor: Position, pub linear: Linear }
```

`Fixed` is the signed integer type behind `Axis` (the wrapper over the big-integer crate, with the inline fast
path). On the wire `RelOffset` is two `axis`-style numbers without the `frac` split (a length-prefixed signed
magnitude), at most 2 x 21 bytes. Fallback if 02 keeps an f64 offset: `MAX_LEVEL_SPAN` must fall to 3, where the
position error is `2^(30 - 53) = 2^-23` of the child's size (about 1e-7); at 4 it is 1e-4, which a viewer who zooms
in on the child can see.

**`compose`: a child through one parent.** Input: the parent's world placement `P = (lp, anchor_p, Lp)` and the
child's `ChildXform`. Output: the child's world placement `W = (lc, anchor_w, Lw)`. Every step is an integer
operation or a fixed-order float operation from the reference list, so the result is the same on every target.

1. `lc = lp + dl`. Refuse if `|lc|` exceeds the depth cap.
2. Validate: the entries of `Lp` and `linear` are finite, and each is 0 or has magnitude in [2^-60, 2^20]. (A group
   scaled outside that range is re-levelled first, as below.)
3. **Rotated offset, exactly.** Write each entry of `Lp` as `m x 2^e` with `m` an `i64` of at most 53 bits (the
   float's exact decomposition). With `o = (ox, oy)` the offset (integers):
   `vx = ma ox 2^ea + mc oy 2^ec` and `vy = mb ox 2^eb + md oy 2^ed`. Each sum is formed as an exact integer: align
   the terms at the smaller exponent `e0` (the multiplications are big integer times 53-bit mantissa), add, and
   keep the value as `V = sum` with scale `2^e0`. The exponents differ by at most 80 (step 2), so the numbers stay
   short. `v` is in steps of 2^-54 child units: an exact real number `V x 2^e0`.
4. **One rounding.** Bring `anchor_p` and `v` (kept at scale `2^e0`, so the sum is exact) to the finer of the two
   levels `f = max(lp, lc)` (a left shift of the
   coarser by `10 x` the difference; exact), add them, then floor to level `lc` (an arithmetic right shift when
   `lc < f`). The floor is the only rounding in the translation: at most 2^-54 child units.
5. `Lw = Lp . Lc` as a 2 x 2 matrix product with the fixed evaluation order below, with no fused operation:
   `w0 = (pa*ca) + (pc*cb)`, `w1 = (pb*ca) + (pd*cb)`, `w2 = (pa*cc) + (pc*cd)`, `w3 = (pb*cc) + (pd*cd)`,
   where `p` is `Lp` and `c` is the child's `linear`, and `m = [a, b, c, d]`. Refuse if any result is not finite.

The units are consistent: the child's `linear` maps child-level geometry to child-level units, `Lp` is
dimensionless, and `off` is already in child-level steps, so the level factor `1024^dl` appears nowhere in the
float part. A deep chain is composed from the root down: `world(n) = compose(world(n-1), xform(n))`, so the order of
rounding is fixed and every machine gets the same bits. The depth is at most 32 (section 02). Total level span along
a chain may exceed 8; each link is at most 8 and the cap bounds the sum.

**`localize`: the inverse, for reparenting and for making a group.** Input: the object's world placement `W` and
the new parent's world placement `P`. Output: a `ChildXform` such that `compose(P, localize(P, W))` equals `W`
except for the roundings stated here.

1. `dl = W.level - P.level`. If `|dl| > 8`, refuse; the caller re-levels the object first (below) or does not
   group it.
2. `D = W.anchor - P.anchor` exactly, at the finer level `f`; shifted to `W.level` steps if `W.level` is the
   coarser (floor).
3. `off = round(Lp^-1 . D)`. `Lp^-1` is `(1/det) [d, -c; -b, a]` with `det = a d - b c` computed in f64. Refuse if
   `det` is 0 or the entries are out of the range of step 2. The product of this float matrix with the integer `D`
   uses the same exact decomposition and single floor as in `compose`.
4. `linear = Lp^-1 . Lw` by the matrix product above.

Round trip: the offset is wrong by at most the floor of one child step, but `Lp^-1` carries float error, up to
about 4 ulps (relative 2^-51) of `|off|`. So ungrouping and regrouping a group whose rotation is not a multiple of
90 degrees or whose scale is not a power of two drifts each child by about 2^-51 of its offset per cycle; section 02
states this and accepts it. For a group with identity `linear` or a power-of-two scale the cycle is exact. A test
measures the error.

**`relevel`.** `relevel(xform, k)` for a `TopXform` is the table above: shift the anchor, scale `linear` by `2^(10
k)`. For a `ChildXform` with a new `dl' = dl + k` (it must stay within 8): `off' = off << 10 k` (exact) or
`floor(off >> 10 |k|)` for coarser, and `linear' = linear x 2^(10 k)`. The world placement of a child does not
change, apart from the same floor as for a top-level object.

**Re-levelling a group does not touch the children.** Writing `TopXform` of a group with `level + k` and
`linear x 2^(10 k)` changes every child's world level by `k` through `lc = lp + dl`. The child's world offset is
`Lp' . off` in steps of level `lc + k`, which is `2^(10 k) Lp . off` steps that are `1024^-k` as long: the same
length. The child's geometry is scaled by `Lp'` in units that are `1024^-k` as large: the same size. So the group is
re-levelled by one write and every descendant is unchanged on the surface.

**Which object is measured.** The size band applies to the **world** size, in world-level units, of the whole
subtree's box. Only a top-level object is re-levelled automatically, by the rule above, using the box of its whole
subtree. A child whose own world size leaves the band (a child scaled alone) is re-levelled alone by changing `dl`;
if that would take `|dl'|` above 8 the change is refused with a message, as grouping objects that far apart is.
A large group is therefore cheap: a group of 10,000 members is moved, rotated, scaled or re-levelled with one
write.


### Hit testing and selection

All pointer work happens in **camera-relative pixel space on prepared geometry**, the same cut pieces that are
drawn. That makes every level uniform, including objects 100 levels above the view whose own lattice is too coarse
to give a hit tolerance of 6 pixels.

1. **Pointer to world.** `P = center + (pointer_px - viewport/2) / zoom` as an exact position (the small f64 offset
   is converted to steps and added). Needed only to hand to presence/server (cursor) and for "create here". The
   pointer is also kept in `R`-relative f64.
2. **Candidates.** The spatial index of the client (rtree over prepared bounds in `R`-relative f64, conservative)
   returns the objects whose bounds, expanded by the tolerance, contain the pointer.
3. **Test.** Distance from the pointer to each prepared segment, in pixels (strokes and lines), or winding
   number of the prepared outline around the pointer for filled shapes, in order of the document's order key, top
   first. For a cut object the winding includes the chord pieces, which is the same correctness argument as
   drawing.
4. **Marquee.** A rectangle in pixels becomes an exact world rectangle (corners as exact positions). "Touches"
   uses the prepared pieces. "Fully inside" uses the object's exact bounds against the exact corners; an object
   larger than the marquee is never fully inside it.
5. **Reachability.** Only objects the client holds can be selected. Section 03 decides what is sent: objects that
   cross the padded view and cover at least `min_px` screen pixels, raised to `eff_min_px` under load. Content
   smaller than that cannot be clicked until you zoom in; the "more here if you zoom in" hint from section 03 is
   the answer for the user.
6. **Ties across levels.** Order is the object order key, regardless of level. A click on a point covered by a
   huge backdrop and a tiny object picks the one with the higher key.

### Images and raster tiles at any depth

Sections 05 (image tiles) and 06 (raster picture tiles) choose what to fetch or draw from three quantities that
would otherwise need big numbers at depth. All three come back as ordinary finite values, computed from the exact
anchor and the exact camera.

```rust
/// How an image is seen now. Both values are finite f64 whatever the depth.
pub struct ImageView {
    /// Screen pixels per source pixel: the larger of the x and y ratios, saturated to [2^-1000, 2^1000].
    pub screen_per_src: f64,
    /// The visible part of the image in image-local source pixels, [x0, y0, x1, y1], clamped to the image
    /// rectangle and rounded OUTWARD. `None` when no part of the image can be on screen.
    pub visible_src: Option<[f64; 4]>,
}
/// `local_rect` is the image's rectangle in its own object units (`[x0, y0, x1, y1]`, so 02 decides where its
/// origin is); `src` is the source size in pixels. `world` comes from `compose`.
pub fn image_view(world: &Placement, local_rect: [f64; 4], src: [u32; 2], cam: &Camera) -> ImageView;

/// The origin (top-left corner) of tile (tx, ty) at tile octave `octave`, minus `origin.at`, in units of the
/// level of `origin.at`. A tile at octave O has side 2^(9 - O) level-0 units (06). Exact, then rounded once.
pub fn tile_origin_rel(octave: i64, tx: &Cell, ty: &Cell, origin: &RenderOrigin) -> [Rel; 2];

impl Scale { pub fn floor_log2(&self) -> i64; }      // 10 x level + floor(log2 z), from the exponent bits
impl ExactBox { pub fn side_bits(&self) -> i64; }    // floor(log2(longer side)) in level-0 units
```

**`screen_per_src`.** The world placement's linear part `Lw` maps object units to units of the world level. Pixels
per object unit along local x is `|Lw (1,0)| x zoom x 2^(10 (camera.level - world.level))`, and along y with
`(0,1)`. The power of two is carried as an exponent and combined with the others before conversion, so there is no
intermediate overflow; the result is multiplied by (object width / source width) for x and the same for y, and the
larger is returned, saturated to the stated finite range. A tile choice only needs the order of magnitude, so
saturation is harmless.

**`visible_src`.** The camera's rectangle is mapped into the image's frame: the exact `Delta` from the world anchor
to the camera center is converted to f64 in object units (if it is `Far`, or larger than 4 times the image's longer
side plus the view's own extent, the answer is `None`), and the viewport half-extent is converted to object units by
the inverse of the scale above. The four corners go through the inverse of `Lw` (the `det` and inverse matrix of
`localize`; singular means `None`), their bounding box is intersected with `local_rect`, and the result is scaled to
source pixels. It is rounded outward by `2^-48` of the largest magnitude involved. This matters at depth: when the
view is smaller than one source pixel by 2^-60 the f64 corners cannot resolve the window itself, and the padded
rectangle is then about one f64 step wide at the right place, which is all a tile choice needs.

**`tile_origin_rel`.** The corner of tile `(tx, ty)` is `tx x 2^(9 - O)` level-0 units. As a position axis value at
level `f` it is `tx << (63 - O + 10 f)` steps, so choose `f = max(level of origin.at, ceil((O - 63) / 10))`, which
makes the shift non-negative. The result is `(tile corner at f) - origin.at.at_level(f)`, an exact `Delta`, then
`Delta::to_f64` in units of the origin's level. For tiles near the view it is `Rel::Near` and small; section 06 adds
the per-frame camera offset to it, exactly as it does for `Visible::Plain`. The size of a tile in pixels needs no
call: it is `2^(9 - O)` times the total scale, and `Scale::floor_log2` gives the octave at which a pixel of the
screen equals a tile pixel.

**`ExactBox::side_bits`.** An `ExactBox` keeps its two exact sides as integers (steps of 2^-54 units of
`min.level`) next to the rounded-up `w` and `h`. `side_bits` is
`bit_length(longer side) - 1 - 54 - 10 x min.level`, with no floating point. A side of zero (a dot) counts as 2^-10
units of its own level, the same rule as section 03's finest row. Section 06's `home_octave` is `2 - side_bits`.

### Bounded surfaces and the depth cap

```rust
pub struct DepthCap {              // always present; an abuse backstop, not a product feature
    pub max_level: i32,            // default 1024      (zoom 2^10240, about 1e3082)
    pub min_level: i32,            // default -1024
    pub max_cell_bits: u32,        // default 12288     (1,536 bytes per axis)
}
pub struct SurfaceLimits {         // optional product limits chosen by the surface creator
    pub bounds: Option<Bounds>,    // content must stay inside
    pub zoom: Option<ZoomRange>,   // the camera must stay in this range
}
pub struct Bounds { pub level: Level, pub min: [Axis; 2], pub max: [Axis; 2] }   // exact rectangle
pub struct ZoomRange { pub min: Scale, pub max: Scale }
pub struct Scale { pub level: i32, pub z: f64 }   // total = z * 1024^level pixels per level-0 unit; z in [1,1024)
```

**The depth cap's default.** A person turning the wheel for an hour at 10 doublings per second reaches 36,000
doublings, 3,600 levels. That would cost a position 9 KB and a cut up to 20 ms per curve (and 3,600 levels of
nothing to look at). I choose **1,024 levels either way** as a default: 10,240 doublings, which is one hour of
hand-zooming at about 2.8 doublings per second, but also bounded: a position is at most 3.1 KB and the worst cut
costs about 15 ms (budgeted). `max_cell_bits = 12,288` allows cell values up to 2^12288: 1,024 levels (10,240 bits)
plus 2,048 bits of horizontal distance. The server refuses a placement or view beyond the cap, and `check_cap_bytes`
decides this on the raw bytes before any number is built; the client camera stops at the cap (a gentle "this is as
deep as this surface goes" indicator). Raising it per surface is possible.

**Bounded surfaces** use the same engine with limits (inspired by tldraw's bounds behaviors):
- *Bounds:* the camera center is clamped so the view keeps overlapping the bounds, with tldraw's behaviors available
  (`free`, `inside`, `contain`, `fixed`). The server rejects placements whose exact bounding box leaves `Bounds`.
- *Zoom range:* the camera clamps `Scale` (total scale, compared lexicographically since `z` is canonical). Objects
  are rejected if their level is outside `[min.level - 2, max.level + 2]`; the slack is the size band's width plus
  one. The check is a safety check; the actual limit is the camera's.
- *Both together* give the equivalent of Excalidraw/Figma-style finite canvases with exact edges.
- Changing limits on an existing surface does not move existing content; content outside is kept and shown, and
  the camera can still pan to it (soft limit) only for roles that can edit limits. This choice is a product
  question for the owner.

### View rectangle for the server, and the interfaces other sections call

**The client sends the exact, unpadded view.** Section 03 owns the wire message (`SetView` carrying `ViewMsg { rect:
ViewRect, min_px }`) and does all padding: half a screen for the sent region, a full screen for the keep region,
one eighth for cursor routing. Nothing on the client side pads. This section provides the rectangle type, the
conservative pixel size, and the function that makes the rectangle from a camera.

```rust
/// An exact axis-aligned rectangle on one level. `min` is rounded DOWN and `max` rounded UP onto the lattice.
pub struct ExactRect { pub level: Level, pub min: [Axis; 2], pub max: [Axis; 2] }

/// What a view sends: the exact rectangle plus the conservative size of one screen pixel.
pub struct ViewRect {
    pub level: Level, pub min: [Axis; 2], pub max: [Axis; 2],   // same fields as ExactRect
    /// floor(log2(world size of one screen pixel, in level-0 units)), from the exponent bits of the camera zoom
    /// (no libm): rounded down, so a size test that uses it can only select too many objects, by at most a
    /// factor of two, never too few.
    pub pixel_log2: i64,
}

impl Camera {
    pub fn view_rect(&self) -> ViewRect;     // the camera's visible rectangle, exact and unpadded
    pub fn scale(&self) -> Scale;            // canonical (level, z) with z in [1, 1024)
}
impl ViewRect {
    pub fn exact_rect(&self) -> ExactRect;
    pub fn scale(&self) -> ViewScale;        // for presence (04), see below
}
impl ExactRect {
    pub fn intersects(&self, other: &ExactRect) -> bool;   // exact; converts the coarser to the finer level
    pub fn contains(&self, p: &Position) -> bool;          // exact, both axes
    pub fn grown(&self, num: u32, den: u32) -> ExactRect;  // each side grows by num/den of the size, outward;
                                                           // growing by half or a full size is a shift
}
```

`Camera::view_rect` computes the visible rectangle `center +- viewport/(2 zoom)`, converts the f64 half-sizes to
steps, rounds `min` down and `max` up, and so contains every point that is truly visible. `pixel_log2` is
`-10 x level - e` where `e` is the exponent of `zoom`, minus one more if the mantissa of `zoom` is above 1. The
server's size test is therefore conservative to within a factor of two (it may send objects down to half a
pixel); section 06 culls precisely with the `eff_min_px` that the server reports.

The camera zoom range is [0.5, 2048) pixels per unit. A view needs no level semantics: it is a rectangle in the
single exact plane, and two clients with different camera levels send the same rectangle (up to representation).

**A view that straddles a level boundary is not a problem.** The question in `design.md` ("how a view that straddles
a level boundary is drawn and subscribed to") assumed the camera's level mattered. It does not. Levels belong to
objects; the camera's level is only the unit it counts in. Drawing a view that touches objects at three levels is
the normal case, handled by per-object conversion. I treat that open point as closed.

**Interfaces named by other sections.**

| Caller | Function | Exact meaning |
|---|---|---|
| 03 | `Position::tile_at(row) -> TileKey` | Level-`L` axis value `n` (steps of 2^-54 level-`L` units) in level-0 units is `n x 2^-(54 + 10 L)`. A tile of side `32^row` level-0 units is `2^(5 row)`. So `tx = n_x >> s`, `ty = n_y >> s` (arithmetic shift, floor) with `s = 54 + 10 L + 5 row`; if `s < 0` it is a left shift. `TileKey { row, tx: Fixed, ty: Fixed }`, hashable and canonical. |
| 03 | `Position::bucket_at(row) -> BucketKey` | The same with side `32^(row+2)`, so `s = 54 + 10 L + 5 (row + 2)`. `BucketKey { row, bx: Fixed, by: Fixed }`. |
| 03 | `Axis::shr_floor(bits: u32) -> Fixed`, `Axis::shl(bits) -> Fixed` | The shift with floor on an axis value (`n >> bits` as a signed floor), and the exact left shift. `tile_at` and `bucket_at` are these. |
| 03 | `Position::offset_from_corner(&BucketKey) -> Option<[i128; 2]>` | Exact `n - corner` in steps of 2^-54 level units of the position's own level; `None` if it does not fit `i128`. This is `ObjBox.off`. |
| 03 | `Position::from_corner_offset(&BucketKey, level: Level, off: [i128; 2]) -> Position` | The exact rebuild: corner shifted to `level`, plus `off`. The inverse of `offset_from_corner`. |
| 03 | `Position::offset_in(&BucketKey, unit_level, Round) -> [f64; 2]` | The same difference converted to f64 in units of `unit_level` with `Round::Nearest`, `Down` or `Up` (directed rounding is the same code with the sticky bit used differently). `Down` for an `Entry.lo`, `Up` for a `hi`, so the stored box is outward. |
| 03 | `ExactBox { min: Position, w: f64, h: f64 }` and `fn exact_box(world: &Placement, outline: &Outline) -> ExactBox` | The exact bounds corner of a world placement's lattice outline, with side lengths rounded up. Section 03 owns the stored `ObjBox` (bucket, level, `off`, `w`, `h`) and builds it from an `ExactBox` with `offset_from_corner`. |
| 03 | `ExactRect::intersects`, `contains`, `grown` | As above. |
| 03, 04 | `Position::sub(&self, &Position) -> Delta` and `Position::add_delta(&self, &Delta) -> Position` | Exact subtraction at the finer level, and its exact inverse. |
| 03, 04 | `Position::encode_delta(&self, base: &Position, out)` / `decode_delta(base, input)` | `zigzag_varint(self.level - base.level)`, then for each axis the signed integer `n_self - n_base`, both taken at `f = max(self.level, base.level)`, written as `varint(len * 2 + sign)` and the magnitude bytes (the whole difference, no separate `frac`). Decoding is `add_delta` then a shift down to `self.level`, exact because the low bits of `self` are zero by construction. A nearby anchor costs 3 to 6 bytes instead of 19 or more. Whole-position fallback: the plain `Position` encoding. |
| 03, 08 | `check_cap_bytes(&[u8], &DepthCap) -> Result<()>` | Reads the level varint and each length varint, and compares them to the cap **before** any number is built. A position over the cap is refused without allocating. |
| 04 | `Position::delta(a, b, unit_level) -> Option<[f64; 2]>` | `Position::sub` followed by `Delta::to_f64(unit_level)`; `None` if either axis exceeds `2^20` units of `unit_level` (so the result has a fixed accuracy of about 2^-33 units) or is `Rel::Far`. |
| 04 | `Position::cell_at_level(l) -> (Cell, Cell)` | The cell indices of a position at level `l`: `at_coarser_floor` (floor) if `l` is coarser, `at_level` (exact) if finer. |
| 04 | `Scale { level: i32, z: f64 }` and `ViewScale { level: i32, q: u16 }` | `Scale` is canonical (`z` in [1, 1024)); total scale is `z x 1024^level` pixels per level-0 unit. `ViewScale` is `Scale` with `z` quantized to `q = round(64 x log2 z)`, 0 to 640, from the f64 exponent bits and a 64-entry table (no libm `log2`), so every client gets the same `q`. Two scales compare as `10 x 64 x (level difference) + (q difference)`. `Camera::scale()` and `ViewRect::scale()` give them. |
| 04 | `QPos` rounding | A `Position` with each offset rounded to a multiple of 2^-14 level units, carrying into the cell: `frac` is rounded to a multiple of `2^(54 - 14) = 2^40` steps. Exact, canonical, and `encode_delta` applies. |
| 04 | `Bounds { level, min: [Axis; 2], max: [Axis; 2] }` and `ExactBox` | `Bounds` is an exact rectangle (surface limits; the union of all content). For "the sender's own box" (anchor plus size) presence should use `ExactBox`, not `Bounds`. |

Views are not sent in presence; they are sync messages (`SetView`). The server reads each connection's view
in-process (03's `RoutedView`) and uses `ExactRect::contains` on the unpadded rectangle (grown by one eighth, in 03)
for cursors.

### Navigation aids

- **Depth readout.** A scale readout from the camera: total scale as scientific notation computed from
  `10 x level x log10(2) + log10(zoom)` in f64 (display only, e.g. "x 2.3e+301"), plus the level number. A small
  scale bar for the current zoom.
- **Home.** `SurfaceMeta.home: SavedPlace`. A new surface's home is level 0, cell (0,0), width 1,000 units. A
  "home" button animates to it; if the owner has not set one, "fit all" is offered instead (needs the extent of
  all content from the server: the exact bounds of the root of the index, a section 03 query).
- **Saved places.** `SavedPlace { id, name, center: Position, half_width: WorldLen }` where `WorldLen { level, m }`
  (half-width of the region to show, `m` in [1, 1024), a device-independent value). Jumping sets zoom to fit that
  width to the viewport (`contain`). Anyone with the editor role can add one; viewers can use them.
- **Jump to a position.** A shareable link carries the canonical position bytes plus the `WorldLen` in base64url in
  the URL fragment (so it is not sent to the server on load). The depth cap bounds link length (up to about 4.2
  KB); links of typical depth are under 400 bytes. The same function powers "jump to this person" using their
  view rectangle.
- **Back/forward.** A history of camera states (after each animated jump, not each pan) with Back and Forward.
- **Lost detection.** If the view contains nothing for more than 30 screens of panning at any scale, or after
  zooming out far with nothing in view, the controls offer "go home" and "go to nearest content" (a server query,
  03: nearest object to a position across all levels, answer an exact `Position`).
- **Context hint (optional).** Name of the largest object that contains the view, if any ("inside: Project plan"),
  obtained from the same cut that detects the camera inside a shape.

### API by name

One place to copy names from. Types come first, then functions; the owner of each is this section unless a
column says otherwise.

| Name | Kind | One line |
|---|---|---|
| `Level` | type | `i32` level; `|level| <= DepthCap::max_level`. |
| `Cell`, `Axis`, `Fixed` | types | Signed integer cell (inline `i64` or big), axis `{ cell, frac: u64 }`, and the integer behind them. |
| `Position` | type | `{ level, x: Axis, y: Axis }`; canonical encoding, 19 bytes at the origin, at most about 3.1 KB. |
| `Delta`, `Rel` | types | Exact difference at the finer level; `Rel::{Near(f64), Far(Sign)}`. |
| `Linear`, `Placement` | types | Matrix `[a, b, c, d]`; world placement `{ level, anchor, xf }`. |
| `TopXform`, `ChildXform`, `RelOffset`, `MAX_LEVEL_SPAN` | types, const | The stored forms (02's register); `ChildXform { dl, off, linear }`; span 8. |
| `LPt`, `Seg`, `Outline` | types | `i64` lattice at 2^-40 object units; |coordinate| < 2^61. |
| `ExactBox` | type | `{ min: Position, w, h }` plus its exact integer sides; 03 owns the stored `ObjBox`. |
| `ImageView`, `image_view` | type, fn | Screen pixels per source pixel and the visible source rectangle, as finite `f64` (05). |
| `tile_origin_rel` | fn | A raster tile's origin minus `RenderOrigin.at`, exact then rounded once (06's `RasterKey`). |
| `ExactBox::side_bits`, `Scale::floor_log2` | fn | Floor of log2 of the longer side in level-0 units (06's `home_octave`); octave of the screen scale. |
| `ExactRect`, `ViewRect` | types | Exact rectangle; rectangle plus `pixel_log2`. |
| `Bounds`, `ZoomRange`, `SurfaceLimits`, `DepthCap` | types | Surface limits and the depth cap. |
| `Scale`, `ViewScale`, `WorldLen`, `SavedPlace` | types | Total scale; its 1/64-octave form; navigation values. |
| `TileKey`, `BucketKey`, `Round` | types | Index keys and the rounding choice for `offset_in`. |
| `Camera`, `CameraLimits`, `RenderOrigin`, `Visible` | types | Camera and drawing caches. |
| `Position::at_level`, `at_coarser_floor` | fn | Exact shift to a finer level; floor to a coarser level. |
| `Position::sub`, `add_delta`, `delta`, `cmp_axis` | fn | Exact difference and its inverse; small difference or `None`; exact comparison. |
| `Delta::to_f64(unit_level) -> [Rel; 2]` | fn | Round to nearest even; ours, not the crate's. |
| `Position::cell_at_level`, `tile_at`, `bucket_at` | fn | Exact cell, tile and bucket of a position. |
| `Axis::shr_floor`, `Axis::shl` | fn | Shift with floor, and exact left shift. |
| `Position::offset_from_corner`, `from_corner_offset`, `offset_in` | fn | Integer and float offsets from a bucket corner, and the exact rebuild. |
| `Position::encode_delta`, `decode_delta`, `check_cap_bytes` | fn | Delta coding against a base; cap check on raw bytes. |
| `ExactRect::intersects`, `contains`, `grown`; `ViewRect::exact_rect`, `scale` | fn | Exact rectangle operations. |
| `Camera::view_rect`, `scale`, `pan_by_px`, `zoom_about`, `zoom_to_fit`, `set`, `clamp`, `normalize` | fn | Camera operations. |
| `compose`, `localize`, `relevel` | fn | Group maths: child through parent; inverse; level change (top-level and child). |
| `exact_box(&Placement, &Outline) -> ExactBox` | fn | Exact bounds of an outline. |
| `QPos` | type | Position with offsets rounded to 2^-14 level units (04's cursor form). |

## Failure modes

| Failure | What happens |
|---|---|
| Hostile position: level or cell beyond the cap, non-canonical bytes | Decoder refuses; server refuses the change and counts it against the sender's rate limit (08). |
| Hostile cost: many huge curves whose `Δ` is large, all crossing the window | The cut queue has a per-frame budget; items past it draw from the previous cut or are marked pending. Also: the server may limit how many objects with `Δ` above a threshold it sends per view (03/08). The cap bounds the worst case at about 15 ms per curve. |
| NaN or infinity from outside | Refused at the boundary (`is_finite`). Stored data that predates a rule is validated on load; an invalid object is kept in the document but not drawn, and reported. |
| `i128` fast path overflow | Fast path is chosen by a bit-length check before arithmetic; the path is tested at the boundary values; on any doubt use the big path. |
| Camera flapping between levels | Hysteresis (0.5 to 2048). |
| Re-base error accumulation | Prepared values are recomputed from exact anchors, never updated incrementally. Test with a million random pans. |
| Concurrent move and re-level (or rotate) of the same object | The `Xform` is one value, last writer wins; the result is a valid placement and one of the two edits is lost (02 states this cost). If it were three fields the merge could pair a new level with an old anchor and move the object by 1024x. |
| Two clients disagree about a lattice outline (a rule violated) | Detected by the cross-target golden test in CI; at runtime, the server's bounds are authoritative, and 03's padding of the view absorbs one lattice step of difference. |
| f32 overflow or poor precision for far geometry | Prepared geometry is clipped to `W''` (about 74,000 pixels); nothing larger reaches the GPU. |
| Zoom to the depth cap | Camera stops, indicator shown. |
| Float drift in camera zoom | Zoom is local and renormalized; position is exact. Drift of `zoom` changes the pixel scale by 1e-16 relative per operation, not visible. |
| Animation to a saved place that was deleted | Animation to a stale target ends at the target position anyway; no index lookup needed. |
| Clock or frame hitches during animation | Animation is a function of elapsed time, not of frame count. |
| Truncated precision of `Delta::to_f64` for values over 2^1000 | `Far`: object is not prepared (it cannot be within `W'`). |

## How it is tested

| Rule | Test |
|---|---|
| A position round-trips and has one encoding | Property test: random `Position`s up to the cap serialize and parse to equal values; every non-canonical mutation of the bytes is rejected. |
| `at_level` is exact; `sub` and `cmp_axis` are correct | Differential test against a second big-integer implementation (`dashu-int` in dev-dependencies) on 1e6 random pairs including levels 0 to 1,024 apart and values near powers of two. |
| `Delta::to_f64` rounds to nearest even, same on every target | Compare against exact rational rounding from the reference implementation; golden vectors run on native and wasm32. |
| Zooming through a level boundary has no jump | Project 1,000 fixed world points before and after `normalize` for random cameras: finer path identical to the bit in the center and exact in the scale; coarser path within 2^-44 pixel. Sweep zoom with the pointer exactly at 2048, 0.5, and ±1 ulp. |
| Pan across any cell boundary is smooth | Pan by one pixel steps over 1e6 steps across cells at several levels: displacement of a fixed point is exactly the step (within 1e-9 px). |
| Zoom keeps the focus point | `zoom_about` then project the focus point: error below 1e-6 pixel at every depth tested up to 1,024 levels. |
| Re-base does not accumulate error | One million random pans; compare every prepared origin with a fresh exact conversion: difference below 1e-9 of a pixel. |
| Camera-relative values are accurate | Random objects at levels -3 to 3 and at depth 1 to 1,000 levels: projected position vs reference rational arithmetic within 0.01 pixel in the window. |
| Cut is faithful | For random curves and `Δ` from 1 to 200: compare the prepared pieces with a reference de Casteljau in rationals with 4,000 bits: control points within 2^-38 of the window side; winding number at 10,000 sample points inside the window identical. |
| Fill is correct when the window is entirely inside | Shapes with the camera inside at `Δ` up to 500: coverage of the window is full; just outside: empty. |
| Chord replacement preserves coverage | Randomized shapes: render coverage by sampled winding with and without replacement; equal in the window. |
| Hit testing across levels | Random pointers over scenes with objects whose levels differ from the camera's by 0, 1, 5 and 50 in either direction: result equals the brute-force reference using exact arithmetic. |
| Determinism, native vs wasm | Golden file: 10,000 random objects (all kinds) to lattice outlines and bounds, checked in on x86-64, aarch64 and wasm32 (wasmtime and a browser engine in CI); all compare bit for bit. |
| Forbidden float operations are not used | `clippy.toml` `disallowed-methods` for `mul_add`, `sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `powf`, `powi`, `hypot`, `f64::min`, `f64::max`, `f64::clamp` in `everink-core::geometry`; CI also checks the build features exclude `relaxed-simd`. |
| NaN and infinity never enter | Fuzz the codec and all tool entry points with NaN, inf, subnormals and huge numbers; none reach shared state. |
| Re-level preserves the shape | For random objects and `k` in [-50, 50]: world-space outline before and after equal exactly for finer, within 2^-54 new unit for coarser. |
| `compose` is exact and deterministic | Differential test against rational arithmetic: world anchor equals the exact value floored once to the child lattice; `Lw` is bit-identical on x86-64, aarch64 and wasm32; chains of depth 1 to 32 with level spans up to 8. |
| `localize` inverts `compose` | Random parents (rotated, scaled, levels apart by up to 8) and children: `compose(P, localize(P, W))` differs from `W` by at most 2^-51 of the offset; exact when the parent's `linear` is a power-of-two scale. Refusals (span above 8, singular matrix) are tested. |
| A group is one write | Move, rotate, scale and re-level a group of 10,000 children: exactly one register changes, and every child's world outline is unchanged for the re-level and transformed as a rigid body for the others. |
| Stored points feed the lattice deterministically | 06's builder, run on random `i32` stroke points and placements on three targets, produces bit-identical `i64` outlines and bounds; the operation list it uses equals this section's list (checked by the same lint configuration). |
| Image view is finite and covers the window | Random images, placements (rotated, levels -500 to 500 apart) and cameras: `screen_per_src` is finite; `visible_src` contains every source pixel whose exact position is on screen (reference rational arithmetic), and is `None` only when none is. |
| `tile_origin_rel` and `side_bits` are exact | Random tiles at octaves -1000 to 1000 and origins at every level: equal to the rational reference after the single rounding; `side_bits` equals the bit length of the exact side, and is the same for one box expressed at any level. |
| Cap checks need no allocation | Hostile byte strings (huge length varints, level beyond the cap) are refused by `check_cap_bytes` with a bounded allocation. |
| Placement merges safely | Property test with two replicas changing level and anchor concurrently (with section 02): the merged placement is one writer's whole value. |
| View rectangle is conservative | Brute-force: every object that exactly intersects the true view is inside `Camera::view_rect`; rounding never excludes. Same view at camera levels differing by 1 and 5 yields rectangles that compare equal after conversion to the finer level, up to the outward-rounding step. |
| Animation ends exactly | After the last frame, `center` equals the target position exactly and zoom equals the target within 1e-12 relative; at mid-path the focus error is below 0.01 pixel. |
| The cap holds | Server tests: placements at cap ± 1 on level and bits; cost test: 1,000 adversarial curves at `Δ = 2,000` keep frame time under budget with items pending. |
| Bounded surfaces | Camera clamps for each behavior; server rejects placements outside `Bounds` at exact edges (1 step in and out). |
| Performance targets | Benchmarks for re-base of 10,000 objects at depth 0 and 100, and per-frame update; recorded against the 1 ms target. |

## Dependencies

| Crate | Version checked | License | Status | Use |
|---|---|---|---|---|
| [`num-bigint`](https://crates.io/crates/num-bigint) | 0.5.1 (updated 2026-07-05) | MIT OR Apache-2.0 | Maintained by rust-num, about 739 million downloads | The big integer behind `BigCell` and `Delta`, behind our own thin wrapper so it can be swapped. `no_std` support not checked (unverified). |
| [`dashu-int`](https://crates.io/crates/dashu) (`dashu` 0.6.2) | 0.6.2 (updated 2026-10-02) | MIT OR Apache-2.0 | Maintained; about 2.5 million downloads; small values are stored inline (docs) | Dev-dependency for differential tests; the candidate swap if the spike shows `num-bigint` allocation is a bottleneck. |
| [`robust`](https://docs.rs/robust/latest/robust/) | 1.2.0 | MIT OR Apache-2.0 | `no_std` option | Not used by default (integer lattice replaces it). Allowed for float-only predicates elsewhere. |
| [`libm`](https://crates.io/crates/libm) | 0.2.16 (2026-03-17) | MIT | Part of `rust-lang/compiler-builtins` | Not used in shared geometry. |
| [`malachite`](https://crates.io/crates/malachite) | 0.14.0 | **LGPL-3.0-only** | Maintained | **Rejected.** LGPL conflicts with embedding under MIT OR Apache-2.0 (goal 4: library first; wasm and static linking make LGPL's re-link obligation hard to meet for embedders). |
| [`rug`](https://crates.io/crates/rug) | 1.30.0 | **LGPL-3.0+** (links GMP, a C library) | Maintained | **Rejected:** same license, plus a C dependency that complicates wasm and cross builds. |
| [`ibig`](https://crates.io/crates/ibig) | 0.3.6 (2022-09-17) | MIT OR Apache-2.0 | No release for four years | **Rejected** as the main dependency: unmaintained. |

Dev tooling for the determinism test (a wasm runtime such as wasmtime) is not a runtime dependency; its license has
not been checked here (unverified). Versions and dates are from the crates.io API on 2026-11-11.

The big-integer needs are small: add, subtract, shifts, compare, bit length, byte conversion, and multiplication of
a big integer by a 53-bit mantissa. The wrapper exposes only those, which keeps a replacement (or a purpose-built
in-house `Fixed`) possible.

## What this assumes of other sections

**Section 02 (document model, merge, undo).**
1. An object's `Xform` (a `TopXform` with level, anchor and linear, or a `ChildXform`) is **one atomic
   last-writer-wins register**, not several. Separate registers allow a merge that pairs one writer's level with
   another's anchor. The cost, stated in 02: a concurrent move and rotate of one object lose one of the two.
2. Groups follow 02: a child is relative to its parent, the group has no child list, and a group move, rotate,
   scale or re-level is one write to the group. `MAX_LEVEL_SPAN = 8` is accepted **only if** the child offset is
   the exact `RelOffset` of this section (a signed fixed-point integer in steps of 2^-54 child units, plus the
   level difference `dl`) in place of 02's `[f64; 2]` and with no `dl` field. With an f64 offset the span must be
   3 or less (see "Groups: compose, localize, relevel"). 02 calls `compose`, `localize` and `relevel` as specified
   here and gets refusals (out of range, span above the limit, singular matrix) as errors to report.
3. Object data is stored so that the lattice outline is derivable from stored numbers using only the allowed
   operations; stored floats are finite and in range, validated on load. Stroke points are 06's `i32` lattice at
   2^-16 unit.
4. Undo of a re-level restores the previous whole `Xform`.
5. Per-surface `DepthCap`, `Bounds`, `ZoomRange`, `home` and saved places live in the document or its metadata.
6. Nothing in this section depends on which library holds the text; with the proposed purpose-built object map the
   registers need only a stamp and a value.

**Section 03 (sync).**
1. The client sends the exact, unpadded `ViewRect` (inside 03's `SetView` / `ViewMsg`); 03 pads it. This section
   supplies `Camera::view_rect`, `ViewRect::exact_rect`, and the exact `ExactRect::intersects`, `contains` and
   `grown`. 03 decides what is sent by on-screen size (`min_px`, `eff_min_px`), not by level, using `pixel_log2`. The
   wire message is 03's `ViewMsg`, which carries a `ViewRect`.
2. The functions in the table "Interfaces named by other sections" (`tile_at`, `bucket_at`, `shr_floor`,
   `offset_from_corner`, `from_corner_offset`, `offset_in` with outward rounding, `sub`, `add_delta`,
   `encode_delta`, `check_cap_bytes`) are provided here. Tile side `32^row` level-0 units and bucket side
   `32^(row+2)` are exact shifts.
3. The server computes object bounds itself from stored data through the object's ancestors (`compose`, then the
   same derivation), using the integer outline. It treats its own bounds as authoritative, and sends ancestors with
   any child, as the world placement needs the whole chain.
4. When a group moves the index recomputes the descendants' boxes (03 says it does); each is a `compose` per
   descendant, an exact operation costing about the same as a re-base entry.
5. A "nearest content to this position, across levels" query and "bounds of all content" query exist.
6. The server limits how many objects whose cut is expensive (a large size ratio between object and view) it sends
   for one view.

**Section 04 (presence).** Cursor positions are `Position` (or `QPos`), the sender's view scale is `ViewScale` (from
`Scale`); "inside the view" is `ExactRect::contains` on the view rectangle that sync holds (03 grows it by 1/8).
`Position::delta` supplies the small exact differences for cursor smoothing, and a remote cursor is drawn relative
to the receiver's render origin.

**Section 06 (rendering).**
1. Delivers `Outline` (lattice, closed or open, fill flag, `|c| < 2^61`) using only the allowed float operations.
2. Accepts `Prepared` geometry: plain objects as local f32 paths with a per-frame translation and scale, and cut
   objects as camera-level, `R`-relative f64 segments already clipped to `W''`.
3. Strokes' width is not part of the cut: the outline already includes it (the outline of a stroke is a filled
   shape).
4. Pending items are drawn as nothing or a placeholder.

**Section 07 (controls).** Shows the depth readout, scale bar, saved places, back/forward, Home, and "lost"
prompts; the readout needs an accessible text equivalent.

**Section 08 (identity, limits).** Enforces `DepthCap`, rate limits on placements, and decides which roles may edit
limits, home and saved places.

## Questions for the owner

1. **Depth cap defaults** (product and cost): 1,024 levels each way (a zoom of about 1e3082), positions up to 3.1
   KB. Is that deep enough as a default? Should it be settable per surface by the surface owner or only by the
   operator?
2. **Paste scale:** keep the apparent size (my default) or the world size? Template placement: apparent size at
   about a quarter of the view width?
3. **Bounded surfaces:** hard limits (the camera stops, the server refuses) or soft (the camera is nudged back)?
   What happens to content that is outside the bounds when an owner shrinks them?
4. **Canvas rotation:** wanted in the first version? It does not change the design but changes the view rectangle
   and gestures.
5. **Reserved to you:**
   - *CRDT library (Loro or yrs).* Nothing in this section depends on it. Under the proposed design the library is
     used only for text inside text objects (02), and positions are plain registers with a stamp.
   - *First app.* A whiteboard exercises bounded surfaces and moderate zoom; a notebook or planning surface may
     never need deep zoom and would make this section's complexity mostly unused. Recommendation: pick the app that
     will use deep zoom, since otherwise this design is paying for a feature the first users never touch; the design
     allows delaying the big-integer path behind the `Small` fast path (levels within about ±6 of origin stay in 128
     bits) if the first app is shallow.
6. **Depth readout wording:** "x 1e301" or the number of levels? (small product choice.)

## Sources

Fetched or searched on 2026-11-11 unless noted.

- Unreal Engine 5 large world coordinates: https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-rendering-in-unreal-engine-5
- Godot large world coordinates: https://docs.godotengine.org/en/stable/tutorials/physics/large_world_coordinates.html
- KSP API, Krakensbane: https://wiki.kerbalspaceprogram.com/wiki/API:Krakensbane
- Star Citizen 64-bit interview: https://gamersnexus.net/gg/2622-star-citizen-sean-tracy-64bit-engine-tech-edge-blending
- Outerra blog (2014): https://outerra.blogspot.com/2014
- Floating the origin (blog): https://frozenfractal.com/blog/2024/4/11/around-the-world-14-floating-the-origin/
- Pad++ paper: https://www.cs.umd.edu/projects/hcil/jazz/learn/papers/chi-94-pad.pdf (not readable through the tool)
- Jazz paper: https://perso.lisn.upsaclay.fr/mbl/Teach/SituatedComputing/2017/papers/ZUI/jazz-uist00-bederson.pdf (same)
- Zooming user interface overview: https://en.wikipedia.org/wiki/Zooming_user_interface (not fetched; found by search)
- Endless Paper: https://apps.apple.com/app/id1294105620
- tldraw camera: https://tldraw.dev/sdk-features/camera
- Excalidraw constants: https://raw.githubusercontent.com/excalidraw/excalidraw/master/packages/common/src/constants.ts
- Figma forum on canvas size: https://forum.figma.com/t/make-the-canvas-infinitive/3159
- Perturbation: https://www.ultrafractal.com/help/formulas/perturbationcalculations.html ,
  https://gbillotey.github.io/Fractalshades-doc/math.html , https://github.com/rust-fractal/rust-fractal-core
- S2 cell hierarchy: https://s2geometry.io/devguide/s2cell_hierarchy
- H3 resolution table: https://h3geo.org/docs/core-library/restable
- Slippy map tilenames: https://wiki.openstreetmap.org/wiki/Slippy_map_tilenames
- Shewchuk robust predicates: https://www.cs.cmu.edu/~quake/robust.html (not fetched)
- `robust` crate: https://docs.rs/robust/latest/robust/
- WebAssembly nondeterminism: https://github.com/WebAssembly/design/blob/master/Nondeterminism.md
- WebAssembly numerics: https://webassembly.github.io/spec/core/exec/numerics.html (first 100,000 of 150,000 characters read)
- Relaxed SIMD: https://github.com/WebAssembly/relaxed-simd/blob/main/proposals/relaxed-simd/Overview.md
- Rust float semantics RFC 3514: https://rust-lang.github.io/rfcs/3514-float-semantics.html
- van Wijk and Nuij, Smooth and efficient zooming and panning, InfoVis 2003: https://researchr.org/publication/WijkN03/related
  (abstract level only)
- crates.io API, 2026-11-11: https://crates.io/api/v1/crates/num-bigint , /dashu , /malachite , /rug , /ibig , /libm
- dashu-int docs: https://docs.rs/dashu-int/latest/dashu_int/

**Unverified claims.**
- Pad++, Jazz, Piccolo and Endless Paper internals and depth limits (papers unreadable; store listing says nothing
  about representation).
- Figma's zoom limits and the -65,500 to +65,500 canvas (forum report only).
- Outerra and Star Citizen details beyond the quoted remarks; the exact KSP floating-origin mechanism.
- That Rust never contracts `a * b + c` into a fused operation (the RFC implies it; I found no explicit rule).
- Whether native and wasm32 `sin`, `exp` and similar functions give identical bits (assumed not).
- Flush-to-zero of subnormals on some 32-bit ARM NEON configurations.
- `num-bigint` 0.5.1 `no_std` support; wasmtime's license.
- Timing estimates for big-integer operations, re-base, per-frame update and cut cost: these are my estimates
  from operation counts, not measurements.
- The claim that the van Wijk path keeps the interpolated-center error under a thousandth of a pixel (follows from
  the path shape; to be proven by the animation test).
- The accuracy bound for the cut (2^-38 of the window side): derived by hand from the rounding rule, to be confirmed
  by the cut test.
