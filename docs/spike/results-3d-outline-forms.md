# Spike slice 3, part d: lighter outline forms that leave no holes

Date: 2026-11-13. (Later: the owner chose runs with smoothed sides that day, and the builder was then made the
only one, as `everink_core::outline::build`. The other builders, `build_runs_within` and its arguments, and the
trial's `coverage`, `union` and `picture` runs named below were removed in that change. They are at commit
`4b84a65`: check it out to run the commands in this file.) Code: `crates/everink-core/src/outline.rs` (`build_runs`, `build_runs_within`), its tests in
`crates/everink-core/tests/outlines.rs`, and `spike/render-trial` and `spike/render-web`, which draw the new forms.
Design: [`../design/06-rendering-and-ink.md`](../design/06-rendering-and-ink.md), "The outline: variable width,
deterministic".

Slice 3 left two outline forms and a gap between them. The design's single path is light and leaves holes where a
stroke doubles back or curls ([3a](results-3a-wasm-outlines-cut.md)). Discs and bands leave none and cost the
renderer about six times as much ([3b](results-3b-renderer.md), [3c](results-3c-browser.md)). On 2026-11-13 the
owner chose to have the lighter hole-free forms built and measured before choosing. This part does that. It
chooses nothing: the outline form is the owner's decision. (The owner chose later that day, from this file: runs
with smoothed sides, a run ended at an eighth of a turn.)

## Answer

**There is a form that leaves no holes and costs the renderer about 1.2 times what the single path does, where
discs and bands cost about 6.5 times.** It is called *runs* here. A stretch of the stroke that turns gently is one
closed path, as in the single path; where that path would fold over itself, the stretch ends and another begins.
Its golden file, written on aarch64, passes byte for byte on wasm32 and, in CI, on x86-64.

| | Single path (the design's) | Discs and bands (3a) | Runs, cut at an eighth, sides smoothed |
|---|---|---|---|
| Holes over the stroke | yes | none | none found; proved for straight sides, argued and tested for smoothed ones (below) |
| Segments a stroke, the trial's scene | 165 | 1,485 | 219 |
| Redraw of 10,000 strokes, natively, `vello_cpu` | 61.1 ms | 393 ms | 76.2 ms |
| The same in Chrome, with SIMD | 83.0 ms | 561 ms | 101 ms |
| One tile of 327 strokes in Chrome | 2.1 ms | 14.0 ms | 2.5 ms |
| Same bytes on aarch64, x86-64 and wasm32 | yes | yes | yes |

- **Three variants of runs were built**: a run ended where the stroke turns a quarter turn or more, the same at an
  eighth of a turn, and the second with the sides of each run smoothed as the single path's are. They cost within
  6% of one another. [The pictures](#the-pictures) show why the third is the one to look at: with straight
  sides a stroke looks cut from card when it is magnified, and a limit of a quarter turn leaves spikes at corners.
  `build_runs` is the first of the three, the quarter turn.
- **A true union was tried with a polygon-clipping library and is worse on every count**: 544 segments a
  stroke, 128 ms to redraw, and 1,641 microseconds to make one stroke's outline, against 7.9 for a
  longer and rougher stroke as runs.
- **Fewer and straighter corners** (octagons and bands) were measured in parts 3b and 3c: 889 segments, 262 ms.
  They do not close the gap.
- **Runs are not the same shape as discs and bands.** Where the pressure changes from one point to the next,
  runs taper straight between the two widths; discs and bands put a full disc on the heavier point. And at a
  turn inside a run the outer edge cuts the corner a little. On the trial's strokes runs cover 0.4% to 0.7% less
  area. Nothing is missing from the inside of a stroke.
- **One thing found on the way: the renderer draws discs and bands too heavy.** At 2 to 6 pixels wide, `vello_cpu`
  gives a stroke of discs and bands 110% of the coverage its shape has. The single path and runs are drawn at
  99.5% to 102%.

## What was built

### Runs

`everink_core::outline::build_runs_within(points, base_width, min_cos, smooth)`, and `build_runs`, which is the
first variant.

A stroke's centre line is the same as for the other two builders: the points, less repeats, each with a radius
from its pressure. A **cross-cut** at a point is a pair of corners, one on each side, at the radius. Between two
cross-cuts lies a **piece** with four corners. A piece is **sound** if it is a convex four-cornered shape going
anticlockwise; that is worked out on the outline lattice in whole numbers, so the answer is the same on every
machine.

A **run** is a chain of sound pieces drawn as one closed path: the right-hand corners forward, the left-hand
corners back. Inside a run, the cross-cut at a point leans with the turn there (it is on the normal of the two
pieces taken together), which is the single path's corner. The builder walks the stroke and keeps a run going
while the turn is under the limit and the next piece is sound. Otherwise:

- If the turn is under the limit and a piece is not sound, the run ends flat at that point and a new one begins,
  square to its first piece. A **wedge**, a triangle of the point and the two outer corners, fills the gap the two
  flat ends leave on the outside of the turn. This is what happens along the inside of a tight curl under a wide
  brush, where the single path folds over and leaves its holes.
- If the turn is at the limit or over it, the same, with a **disc** as in discs and bands and no wedge.
- The stroke's two ends have a disc each.

**Why there are no holes.** A run's path is the sum of the edges of its pieces: the cross-cuts between
neighbouring pieces are walked once each way and cancel. So the path winds once round every point of every piece
and never the other way. Wedges and discs go the same way round. Under the non-zero rule a point is filled if
the paths wind round it at all, and no path here can take away what another gives. That is the reason discs and
bands have no holes, and it carries over.

**Smoothed sides.** With `smooth`, a corner inside a run becomes a curve from the middle of one edge to the
middle of the next, with the corner as control point; this is how the single path is smoothed. On the inside of
a turn the curve adds a sliver, which winds the same way as the run and can make no hole. On the outside of a
turn it takes a sliver off, so a run no longer covers its pieces exactly, and the argument above holds only up
to those slivers. To keep them away from the middle of the stroke, a corner on the outside stays sharp where the
stroke's own point is not on the far side of the chord the curve replaces. The ends of a run, the wedges and the
discs are as before, so where a run ends there is still a small step.

**What a run costs.** Two segments a point where the stroke is gentle. Seven a point (a four-cornered piece and
a wedge) where the brush is so much wider than the step between points that the inner side folds at every
point. Discs and bands cost twenty a point, sixteen of them curves.

### In the trials

`spike/render-trial` and `spike/render-web` draw three more forms, the three variants. The native trial has
three more runs:

- `coverage [strokes]` compares what each form covers with what discs and bands cover, a stroke at a time. It
  uses no renderer: each path is flattened to a thousandth of a pixel and cut by sample rows, eight a pixel, and
  along each row the covered stretches are exact. It also draws each stroke alone with `vello_cpu` and sets the
  coverage drawn against the shape's own area.
- `union [strokes]` replaces each stroke's discs and bands by their true union, made by `i_overlay` 9.1.0, and
  draws the view from that. It is for this trial only, in pixels of one zoom.
- `picture <file.png> single|runs` writes the two pictures below.

The browser trial keeps the tiles of the last form it drew as its cache. It used to label that check with the
third form's name, which was right only while there were three; it now names the last.

## Measurements

Machine: Apple M4, macOS. The scene is the one of parts 3b and 3c: 10,000 strokes, random walks of 30 to 120
points, 2 to 6 pixels wide at 256 pixels a unit, over a 3840 x 2160 view in 40 tiles of 512 pixels. "Rough
strokes" are the same with a sharp turn, a doubling back, a hairpin or a pause at about one point in fifteen.
In both, the pressure changes at random from point to point, which no pen does; "steady" strokes have one
pressure.

### Segments

| Form | The scene, a stroke | Rough strokes, a stroke | The core's test corpus, 1,000 strokes |
|---|---|---|---|
| Discs and bands | 1,485 | 1,504 | 2,736,720 |
| Octagons and bands (the trials only) | 889 | 901 | not built in the core |
| Single path | 165 | 196 | 346,997 |
| Runs, cut at a quarter turn | 201 | 301 | 945,380 |
| Runs, cut at an eighth | 214 | 349 | 1,015,291 |
| Runs, cut at an eighth, sides smoothed | 219 | 363 | 1,046,553 |
| True union (the trial only; all straight) | 544 | | |

The test corpus is the hard case for runs: its brushes are up to 80 times as wide as the step between points,
so the inner side folds at almost every point and nearly every piece is a run of its own (180,328 paths for
140,838 points). Runs are still 35% to 38% of discs and bands there.

### What each form covers

300 strokes, each compared with its own discs and bands (`render-trial coverage 300`). "Inside" is the same
stroke built a tenth, and three tenths, thinner as discs and bands: what a form does not cover of that is
missing well inside the stroke, not at its edge. A stroke is counted if more than a thousandth of a square
pixel is missing.

| Strokes | Form | Area more | Area less | Missing a tenth inside | Missing three tenths inside |
|---|---|---|---|---|---|
| The scene's | Single path | 0.34% | 0.71% | 0.010%, 106 strokes | 0.001%, 30 strokes |
| | Runs, quarter turn | 0.16% | 0.43% | 0.011%, 127 strokes | none |
| | Runs, eighth | 0.16% | 0.40% | 0.004%, 101 strokes | none |
| | Runs, eighth, smoothed | 0.34% | 0.65% | 0.003%, 58 strokes | none |
| Rough | Single path | 0.27% | 1.08% | 0.114%, 261 strokes | 0.011%, 106 strokes |
| | Runs, quarter turn | 0.14% | 0.66% | 0.067%, 224 strokes | under 0.0005%, 4 strokes |
| | Runs, eighth | 0.14% | 0.44% | 0.008%, 121 strokes | none |
| | Runs, eighth, smoothed | 0.27% | 0.65% | 0.006%, 96 strokes | none |
| Rough, steady pressure | Single path | 0.14% | 0.64% | 0.083%, 196 strokes | 0.008%, 63 strokes |
| | Runs, quarter turn | 0.10% | 0.38% | 0.050%, 181 strokes | none |
| | Runs, eighth | 0.10% | 0.19% | none | none |
| | Runs, eighth, smoothed | 0.14% | 0.31% | none | none |

Reading it:

- **The single path's holes show as area missing three tenths inside the stroke**, in a third of the rough
  strokes. Runs cut at an eighth have none there in any row.
- **What runs miss a tenth inside is the taper.** At a steady pressure, runs cut at an eighth miss nothing a
  tenth inside. With the pressure changing at random they do, in a third of the strokes, because discs and bands
  put a whole disc on a heavy point and runs do not.
- **A quarter turn is too loose a limit.** At a turn inside a run the outer edge is short of the round join by up
  to the radius times one minus the cosine of half the turn: 29% of the radius at a quarter turn, 7.6% at an
  eighth. With the taper on top of that, four rough strokes miss area three tenths inside (none does at a steady
  pressure); the pictures show the corners.
- **Smoothing moves about a quarter of a percent of area** on these strokes: off the outside of turns and onto
  the inside, all of it at the edge.

Octagons and bands are left out of this table: the trial builds them with a pressure curve that is only near the
core's, so their area is not comparable.

### The pictures

Parts of rough strokes, magnified, drawn by `vello_cpu`
(`render-trial picture <file.png> single` and `... runs`). In both pictures a row is one stroke, the same part of
it in every column, and each cell carries its column's number:

| 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|
| Discs and bands | Single path | Runs, cut at a quarter turn | Runs, cut at an eighth | Runs, cut at an eighth, sides smoothed |

On this page the pictures are shown at about half their size; opened by themselves they are 1572 pixels wide.

**Where the single path does worst.** The three strokes, of the first 300 rough ones, where the single path
leaves the longest stretch of the stroke's inside uncovered; each row is centred there. It is at 8192 pixels a
unit, thirty-two times the scene's zoom.

![Three rough strokes, a row each, in five outline forms, columns numbered 1 to 5: discs and bands, single path, runs cut at a quarter turn, runs cut at an eighth, the same smoothed. Column 2 has a white crescent-shaped hole in the middle of the second row and a thin white slit at the top of the round end in the third; the other columns are filled in the second row and have only a shallow dip in the third. Column 3 has pointed corners where columns 1, 4 and 5 are round.](images/3d-forms.png)

- Column 2, the single path. Row 2: a crescent-shaped hole in the middle of the cell, where the stroke curls
  back under itself. Row 3: a white slit in the top of the round end at the left, where the others have only a shallow dip. In row 1 its outline is a different
  shape from the others where the stroke turns back at the left.
- Column 1, discs and bands: whole. Its edge is lumpy, because the disc of every point shows where the pressure
  changes.
- Column 3, runs cut at a quarter turn: whole, with a point where the others are round: the bottom corner in
  row 1, the bottom of the round end in row 3, and a sawtooth down the left edge in row 2.
- Column 4, runs cut at an eighth: whole. The points of column 3 are gone (row 1, row 3). Its sides are straight
  from point to point, and where a run ends its flat end shows as a small step, as on the right-hand edge in
  row 2.
- Column 5, the same smoothed: whole, with curved sides on both the outside and the inside of turns. The steps
  where runs end are still there (the right-hand edge in row 2).

**Where runs do worst.** No stroke in the first picture was chosen against runs. In this one the three strokes
are those where the smoothed runs (column 5) cover the least of what discs and bands (column 1) cover: 1.7%,
1.7% and 1.6% less area. Each row is centred on the longest stretch they miss. This picture is at 4096 pixels a
unit, half the zoom of the first, so that both edges of a stroke are in the cell.

![Three more rough strokes in the same five columns, at half the zoom. In the first and third rows each stroke is a wide band with both edges in view. Column 1 is the fullest and its edges are lumpy. Columns 3, 4 and 5 are a little narrower in places, with small V-shaped notches in their edges, cut from outside; column 5 has curves where columns 3 and 4 have corners. Column 2 has thin white slits inside the ink near the top of the first row and at the top left of the third. The second row is a thin stroke at its whole size and looks alike in all five.](images/3d-forms-runs.png)

- Rows 1 and 3 show the taper. Discs and bands (column 1) bulge round each heavy point; runs (columns 3 to 5) go
  straight from one width to the next and stay inside that bulge, so they are a little narrower in places.
- The small V-shaped notches in the edges of columns 3 to 5 (row 1, both edges; row 3, the upper edge and the
  bottom) are where one run ends and the next begins: two flat ends meeting at a gentle turn, with a wedge or the
  next run's first piece between them. They are cut into the edge from outside. None is a gap inside the ink.
  Smoothing (column 5) rounds the sides between them and leaves them.
- Column 2, the single path, tapers as runs do, and has slits inside the ink where it folds: near the top of
  row 1 and at the top left of row 3.
- Row 2 is a thin stroke at its whole size: the forms differ a little at its turns, and none has a hole.

In both pictures the edge of column 1 has short white ticks along it. They are the places where the straight
edge of a band meets a disc of another size, and they are part of that shape, not a fault of the drawing.

At the scene's own zoom these strokes are 2 to 6 pixels wide and the forms cannot be told apart by eye, but for
the holes. Everink zooms without limit, so the magnified look matters. The pressure in these strokes changes at
random from point to point; a real pen's changes slowly, and the taper and the lumps would both be milder.

### Redrawing the whole view, 10,000 strokes

Natively (`render-trial 10000`), the median of five redraws after one that is not counted:

| Form | Segments a stroke | `vello_cpu` | ... a tile | `vello_gpu` (Metal) |
|---|---|---|---|---|
| Discs and bands | 1,485 | 393 ms | 9.8 ms | 426 ms |
| Octagons and bands | 889 | 262 ms | 6.6 ms | 293 ms |
| Single path | 165 | 61.1 ms | 1.5 ms | 84.8 ms |
| Runs, quarter turn | 201 | 72.1 ms | 1.8 ms | 92.1 ms |
| Runs, eighth | 214 | 73.9 ms | 1.8 ms | 95.9 ms |
| Runs, eighth, smoothed | 219 | 76.2 ms | 1.9 ms | 95.8 ms |
| True union (`render-trial union`, an earlier session the same day) | 544 | 128 ms | 3.2 ms | not run |

In Chrome 157 on the same machine, the build with WebAssembly SIMD (the web build requires it; owner decision of
2026-11-13), one session in a fresh window with an empty profile:

| Form | `vello_cpu` | WebGL2 | WebGPU | One tile, `vello_cpu` |
|---|---|---|---|---|
| Discs and bands | 561 ms | 555 ms | 553 ms | 14.0 ms |
| Octagons and bands | 366 ms | 366 ms | 385 ms | 9.2 ms |
| Single path | 83.0 ms | 78.1 ms | 80.5 ms | 2.1 ms |
| Runs, quarter turn | 96.5 ms | 93.8 ms | 97.9 ms | 2.4 ms |
| Runs, eighth | 100 ms | 96.4 ms | 103 ms | 2.5 ms |
| Runs, eighth, smoothed | 101 ms | 98.9 ms | 101 ms | 2.5 ms |

The three forms measured before are within 4% of part 3c's figures (540, 358 to 363 and 80 to 81 ms there);
a session an hour earlier, before the smoothing was changed, had them at 536, 360 and 80.6 ms and the two
straight-sided runs at 94.5 and 98.0 ms. The panning part of the page was left out of these sessions (`pan=no`):
a session with it did not finish, most likely because the window was not being shown, and a page that is not
shown gets no animation frames. A session earlier the same day, on a busier machine, did run it over the tiles of
runs: a median of 16.67 ms between frames on both backends, as in part 3c.

Section 06 budgets 4 ms of processor time a frame for tile raster. One tile of 327 strokes takes 2.0 to 2.1 ms
in Chrome as the single path (2.0 in part 3c and in the earlier session, 2.1 in this one) and 2.4 to 2.5 ms as
runs, of any of the three variants. So one whole tile of runs fits a 4 ms box: 40 frames (0.67 s) to redraw the
whole view. Two tiles of the single path fit only just, or just do not: 20 frames (0.33 s) if they do. Discs
and bands take about 140 frames (2.3 s). Natively a tile of runs takes 1.8 to 1.9 ms, and two fit the box.

The three renderers drew the same pixels for every form, as in part 3c: no pixel of the busiest tile differed by
more than an eighth of full coverage between any two.

### Making the outline

`cargo run --release --example bench_cut`: one stroke of 200 points, a random walk that changes direction and
pressure freely at every point (rougher than the scene), 2,000 builds, the median.

| Builder | Time | Segments |
|---|---|---|
| Discs and bands | 7.3 microseconds | 3,996 |
| Single path | 4.5 microseconds | 1,109 |
| Runs, quarter turn | 7.9 microseconds | 2,813 |

Runs take a little longer to build than discs and bands on this stroke: fewer segments, and a soundness test in
128-bit whole numbers for every piece. The outline of a stroke is built once, when the stroke is made or loaded,
not at every redraw.

### A true union

`render-trial union 10000`. Each stroke's discs and bands are flattened to a hundredth of a pixel at the scene's
zoom and handed to `i_overlay`, which returns the outline of their union under the non-zero rule.

- Making the unions: 1,641 microseconds a stroke, 16.4 s for the 10,000.
- 544 segments a stroke, all straight: the round outside of a stroke becomes many short edges.
- It covers what discs and bands cover: 0.027% less area over the first 300 strokes, none more.
- Redraw: 128 ms with `vello_cpu`.

It was not taken further. It is heavier than runs to draw, takes some two hundred times as long to make, and
is made for one zoom; an outline for every zoom would need curves or finer edges, so more segments. Whether the
library gives the same bytes on every machine was not looked at.

## Findings

1. **Runs close the gap slice 3 found.** No holes on any stroke tested (by construction with straight sides; see
   Finding 3 for smoothed ones), at 1.18 to 1.25 times the single path's redraw time natively and 1.16 to 1.22
   times in Chrome, where discs and bands are at 6.4 and 6.8 times.
2. **The limit on a turn should be an eighth, not a quarter.** It costs 2 ms in 72 natively and removes the spikes
   and the area missing inside the stroke.
3. **Smoothed sides are what make runs look like ink when magnified.** They cost 2 ms in 74 natively
   and 1 ms in 100 in Chrome. They also weaken the argument for "no holes" from exact to "exact but for slivers at the outside of turns, kept away
   from the centre line"; the tests check the centre line with the curves followed.
4. **Runs change the shape of a stroke where the pressure changes.** They taper; discs and bands bulge. Neither
   is what section 06 specifies in so many words: its single path tapers too. If runs are chosen, the taper is
   the shape, and the cut (slice 3a), the hit test and the eraser must agree with it.
5. **Under a very wide brush runs lose most of their advantage** (the test corpus: 35% to 38% of discs and bands,
   not 14%). A marker many times wider than the step between its points would be better served by leaving out
   points that add nothing, which no builder here does.
6. **The renderer adds the coverage of overlapping parts of one path at their edges.** A stroke of discs and
   bands 2 to 6 pixels wide is drawn with 110% of its area; eight times as large, 100.3%. So the form built to be
   exact is the one drawn least exactly at ordinary sizes, and thin strokes drawn with it look bolder than they
   are. Runs overlap only at wedges, discs and self-crossings: 100.2% to 101.9%.
7. **A true union is not worth its cost here** (above).

## What this part does not show

- **Real pen strokes.** Every stroke here is a random walk with random pressure. Real strokes are smoother and
  their pressure changes slowly, which favours runs (longer runs, less taper), but that is not measured.
- **How it looks to a person at ordinary zoom.** The two pictures are of six strokes, magnified, chosen as the
  worst for one form or another.
- **The live preview.** Section 06 wants the outline built as the stroke grows. A run can grow at its end; that
  entry point is not written.
- **Other browsers and a slower machine**, as in part 3c.
- **Sub-pixel and very large strokes**, and strokes cut by the eraser.
- **How far smoothing pulls the edge in at a turn.** Found in review after the owner's choice, and not in the
  tables above: the 7.6% is for straight sides. With smoothed sides the curve at an outer corner passes halfway
  between the corner and the chord of its two neighbouring edge midpoints, so on a thin stroke with long steps
  the edge at a turn can come in much further: 44% of the radius at a 40 degree turn between steps of five
  radii, and up to about half the radius before the guard stops it. The single path does the same. On the
  trial's strokes, whose steps are about as long as the stroke is wide, the area figures above include it.
- **A proof for the smoothed variant.** The argument is given above with its limit; the test covers the centre
  line of 1,000 rough strokes at every eighth point, not every point of every stroke.

## Where the code departs from the design

1. Runs are a third builder beside the two of slice 3a, in `everink-core`, for the trial. None is "the" builder
   until the owner chooses.
2. The limit on a turn is passed as a cosine and compared with a dot product of unit directions, in floats like
   the rest of the outline arithmetic; the soundness of a piece is decided in whole numbers on the lattice.

## What the design should change

Nothing until the owner chooses. (Chosen on 2026-11-13: runs with smoothed sides.) If runs are chosen, section 06's steps 4 to 8 describe them in place of the
single path, with the taper stated as the stroke's shape, and the performance budget takes the figures above.

## For the owner's choice

The owner chose the last row on 2026-11-13. The forms that were on the table, with what each means:

| Form | Holes | Redraw, natively | In Chrome | Whole view refilled in Chrome, at 4 ms a frame | Shape and look |
|---|---|---|---|---|---|
| Single path, as designed | yes | 61.1 ms | 83.0 ms | 20 frames (0.33 s) if two tiles fit a frame, which at 2.0 to 2.1 ms each they only just do; 40 if not | Smooth; holes where a stroke turns back or curls |
| Discs and bands | none | 393 ms | 561 ms | about 140 frames (2.3 s) | Exact union; lumpy where pressure changes; drawn 10% heavy at small sizes |
| Runs, eighth, straight sides | none | 73.9 ms | 100 ms | 40 frames (0.67 s) | Tapers; straight edges and small steps show when magnified |
| Runs, eighth, smoothed sides | none found | 76.2 ms | 101 ms | 40 frames (0.67 s) | Tapers; curved sides; "no holes" argued up to slivers at the outside of turns and tested on the centre line |

The refill column is what a person would notice after a large zoom change: whole tiles that fit section 06's
4 ms of tile raster a frame, for a view of 40 tiles of 327 strokes each. One tile of runs fits a
frame. Two of the single path's fit only if a tile stays at 2.0 ms, so by this measure runs take between the
same time and twice as long to refill the view, where the redraw times alone say 1.2 times.

## Kept afterwards

(Done since: see the note at the top of this file.) `build_runs` and `build_runs_within` stay in `everink-core` beside `build` and `build_single_path` until the
outline form is chosen; the builders not chosen go then, with their golden files. `spike/render-trial` stays as
part 3b left it.
