# Spike slice 3a: wasm32, stroke outlines, and the cut

Date: 2026-11-12. Code: `crates/everink-core` (`outline`, `cut`), `tools/wasi-run.mjs`.
Design: [`../design/06-rendering-and-ink.md`](../design/06-rendering-and-ink.md), "Outline algorithm", and
[`../design/01-positions.md`](../design/01-positions.md), "The cut".

Slice 3 answers five questions, and like slice 2 it lands in parts. This first part is the arithmetic: questions
2 (wasm32), 3 and 4 of the [plan](README.md). The renderer (question 10) is part 3b; pen latency (question 11)
is the owner's to run.

## Answers

- **Question 2: positions are the same on wasm32.** The golden file written on aarch64 (1,988 lines) passes
  byte for byte on wasm32, as it did on x86-64. All of `everink-core`'s tests run there.
- **Question 3: stroke outlines are bit-identical** on aarch64, x86-64 and wasm32: one golden file of 1,000
  strokes, written on aarch64, passes on the other two. On the way the spike found that **the outline the
  design specifies leaves holes over the stroke**, and built one that does not ([Finding 1](#findings)).
- **Question 4: the cut costs 0.37 ms for one curve 100 levels above the view and 11 ms at 1,000 levels**, on
  an Apple M4; about three to four times that on wasm32. Against the 2 ms a frame the design budgets, one curve
  fits up to about 300 levels and does not fit beyond, so the cut of a single curve has to be able to stop and
  go on in the next frame ([Finding 2](#findings)).

## What was built

| Part | What it is |
|---|---|
| The wasm32 target | `wasm32-wasip1` in `rust-toolchain.toml`; `tools/wasi-run.mjs` runs a test binary under Node's WASI, and `.cargo/config.toml` makes it the target's runner. `cargo test -p everink-core --target wasm32-wasip1` needs nothing but Node |
| `outline::build` | A stroke's stored points to a closed, filled path on the 2^-40 lattice: a disc at every point and a band along every piece between two points, all going the same way round |
| `outline::build_single_path` | The outline as section 06 first specified it, kept to record its flaw |
| `cut::cut_quad` | One quadratic curve of any size cut to the window, in whole-number arithmetic |
| `crates/everink-core/clippy.toml` | The float rules of section 01 as a lint: `mul_add`, `sin`, `cos`, `powf`, float `min`/`max`/`clamp` and their like are refused in this crate |
| `Axis::to_i128`, and a faster `Axis` comparison | Comparing two long coordinates no longer copies them |

## Question 2 and 3: the same on every machine

| Target | Positions golden file (1,988 lines) | Outlines golden file (1,000 strokes) | Where |
|---|---|---|---|
| aarch64 macOS | written here | written here | Apple M4 |
| wasm32 (`wasm32-wasip1`, Node 26, V8) | passes | passes | Apple M4 |
| x86-64 Linux | passes | passes | GitHub's `ubuntu-24.04`; the pull request that added this slice records the run |
| wasm32 on x86-64 Linux | passes | passes | the same run |

The outlines file holds, for each stroke of a fixed corpus, the number of segments and a hash of every
coordinate, for both builders. The corpus is a random walk with what an outline has to survive mixed in: sharp
turns, doubling back, a pause on one point, no pressure, a single dot, and strokes at the edge of the 32-bit
point lattice. Its pressure changes at random from point to point, which is rougher than a real pen.

**How the outline stays identical.** It uses only `+ - * /` and `sqrt` on `f64`, comparisons, and
`round_ties_even`; its tables (pressure to width, and the cosines and sines of the round shapes) are literals;
the result is rounded to the lattice as the last step. x86-64 has a fused multiply-add and the compiler did not
use it: the file passes there. That settles, for this toolchain, the item section 01 lists as unverified
("FMA contraction by rustc"). It is a test and not a guarantee: a later compiler could change it, and the test
is what would notice.

**What wasm32 means here.** `wasm32-wasip1` under Node is the same WebAssembly arithmetic a browser runs, in the
same engine as Chrome's. It is not a browser: no browser was driven in this part. Other engines (Firefox,
Safari) run the same instructions under the same specification; they are left to part 3b, which needs a browser
anyway.

## Question 4: what the cut costs

`cargo run --release --example bench_cut`. Apple M4 native, and the same binary on wasm32 under Node 26 on the
same machine. For each row, 200 random curves that pass through the window, each cut five times and its fastest
kept; the table gives the median and the 95th percentile over the 200. A curve "N levels above" is about
1024^N times the window across.

| Levels above the view | Halvings | Native, ordinary | Native, worst case | wasm32, ordinary | Design's estimate |
|---|---|---|---|---|---|
| 1 | 11 | 1.8 µs (2.0) | 2.9 µs (3.2) | 6.6 µs (8.7) | |
| 3 | 35 | 4.7 µs (5.0) | 8.1 µs (8.4) | 17 µs (18) | "tens of nanoseconds per segment" |
| 10 | 99 | 20 µs (21) | 37 µs (39) | 55 µs (74) | |
| 30 | 299 | 96 µs (99) | 195 µs (203) | 263 µs (346) | |
| 100 | 1,003 | 372 µs (393) | 701 µs (728) | 1,122 µs (1,423) | 30 to 60 µs |
| 300 | 3,003 | 1.6 ms (1.7) | 2.9 ms (3.0) | 5.0 ms (6.3) | |
| 1,000 | 10,003 | 11 ms (12) | 19 ms (20) | 42 ms (74) | about 15 ms at 2,000 levels |

- **Ordinary** is a curve that crosses the window somewhere along its length: each halving leaves the crossing
  in one half. **Worst case** is a curve that crosses exactly at its middle, so both halves reach the window
  and the work doubles. The halvings column is for the ordinary case: ten per level, as the design expected.
- A curve that misses the window costs nothing to speak of: its box is compared with the window once.
- The cut gives about seven pieces for a curve through the window, at every depth.
- The wasm32 figures were taken with the machine less quiet than for the native ones, and their spread is wider.

**Against the 2 ms budget** (native, ordinary case): 5 curves a frame at 100 levels, 1 at 300, and at 1,000
levels one curve takes five frames' worth. On wasm32, 1 or 2 at 100 levels and none at 300.

**Against the design's estimates:** 6 to 12 times slower at 100 levels. The design's "10 halvings per level, of
which the flatness shortcut makes half cheap" holds; what was underestimated is the cost of one halving of a
long number, about 0.37 µs at 100 levels here. The code is not tuned: every addition and shift makes a new
number. Two things were tried. Taking a piece's bounds without copying its coordinates changed little.
Comparing two long coordinates without copying them made the cut about five times faster (1.75 ms to 0.37 ms
at 100 levels); the copy had been there since slice 1. In-place arithmetic was not tried.

**The stroke outline** (200 points): 7.4 µs native and 20 µs on wasm32 for the discs and bands (3,996 segments);
4.2 µs and 12.5 µs for the single path (1,109 segments).

## What the tests show

`cargo test -p everink-core` now runs 34 tests natively (33 and one skipped on wasm32, which cannot catch a
panic).

- **Outlines (4 tests).** The golden file. Every path of an outline is closed, and nothing strays further from
  the points than the brush's width. Every eighth stroke point, and the middle of the piece after it, is inside
  the outline, and no path goes the other way round (the winding count is positive, never zero or negative):
  more than 30,000 checks. A dot is a circle to within a lattice step. More pressure is never thinner; the lightest touch
  is a quarter of the width and none from the device is the full width.
- **The single path's holes (1 test).** On the same corpus, 10,012 of 140,838 stroke points (7.1%) are outside
  the single-path outline of their own stroke, in 560 of the 1,000 strokes. The test asserts that it is still
  more than 3%, so that a repair of the single path is noticed.
- **The cut (5 tests).**
  - A curve built to pass through a point of the window, from 1 to 500 levels above, is cut to pieces that pass
    through that point to within a millionth of a unit and, from five levels up, lie on the curve's straight
    line there to within a hundred-thousandth.
  - A curve a few windows across is cut to what plain floats give.
  - A curve that misses the window leaves only its chord, with no halving.
  - A triangle up to 500 levels larger than the window, cut side by side, still winds once round the window's
    centre when the window is inside it (either way round) and not at all when it is outside.
  - A shape's point is moved to the camera's level exactly, a thousand levels up.

Tests were checked to be able to fail: with the cut's halving dividing by four instead of two, two cut tests
failed; with the outline's bands turned the other way round, the coverage test and the golden file failed.

## Findings

1. **The outline section 06 specifies leaves holes over the stroke.** It is one closed path: up the left side,
   round the end, down the right side. Where a stroke doubles back, or curls tightly under a wide brush, the
   two sides cross, and the non-zero rule, which the design chose so that no polygon clipping is needed, leaves
   the crossed part empty. In the corpus 7.1% of the stroke's own points are uncovered. With the turns taken
   out of the corpus it is still 1.7%; the rough pressure is not the cause (the count is the same with a
   constant pressure). Small handwriting with a wide marker would show it.
   The prototype's `build` instead makes the union of simple pieces that all go the same way round: a disc at
   every point and a band along every piece. Overlaps then add and never cancel, so there are no holes, by
   construction and by test. The cost is 3.6 times the segments (3,996 against 1,109 for 200 points) and under
   twice the time to build. What that costs the renderer is for part 3b to measure; if it is too much, the
   discs can be thinned where the stroke runs straight.
2. **One curve's cut does not fit a frame from about 300 levels up, so the cut must be able to stop
   part-way.** Section 01 queues the cut by object ("an object whose cut is not finished is drawn from its
   previous cut") and budgets 2 ms a frame. At 1,000 levels a single curve takes 11 to 19 ms natively and 42 ms
   on wasm32. The prototype keeps its unfinished pieces on an explicit stack, so stopping and going on is a
   small change; it is not built.
3. **The design's cost estimates for the cut are low by about ten times at 100 levels.** Its figure at the depth
   cap ("about 15 ms, budgeted") is close: 11 to 19 ms at 1,000 levels.
4. **wasm32 is about three to four times slower than native for this arithmetic** under Node 26 (long whole
   numbers: 3 times at 100 levels, 4 at 1,000; the outline's floats: 2.7 times). Section 01 had no figure. The
   re-base and per-frame figures of slice 1 were not re-run on wasm32.
5. **A comparison in slice 1's code copied both numbers.** `Axis`'s ordering made two big integers to compare
   them. Fixed here. It was not in slice 1's timed paths (subtraction, re-base and the frame do not compare),
   so those figures stand.
6. **A test merged in slice 2c was flaky**, and failed once here.
   `without_keeping_writes_during_the_fill_a_late_joiner_loses_them` queued the room's objects in the order of
   a hash map, which changes from run to run, and its outcome depended on that order. It now sends them in the
   order they were made. `PROFILE.md` said no flake was expected; this was one.

## What this part leaves out

- **The outline:** cuts (partial erase), a taper at the ends, width from speed when there is no pressure, the
  256-entry table (17 entries, interpolated, are used), the incremental builder, and a check against exact
  rational arithmetic. Curves are quadratic only.
- **The cut:** stopping and resuming (Finding 2), cubic curves, the "is the camera inside this shape" answer as
  a call of its own (the winding test shows the pieces carry it), the queue and its budget, the cache by
  re-base, and the check against 4,000-bit rationals that section 01 names (the tests check against the curve's
  construction instead).
- **wasm32 in a browser, and other engines.** Part 3b.
- **`everink-sync` on wasm32.** Its example uses sockets, which that target does not have, so only
  `everink-core` is tested there.

## Where the code departs from the design

1. **The outline is discs and bands, not one path** (Finding 1). Its paths are mostly curves through sixteen
   corners per disc, and straight edges for the bands.
2. **The pressure table has 17 entries**, every one a multiple of 1/64, interpolated; section 06 asks for 256.
3. **A far piece is pulled onto a box sixteen windows across** and kept as a chord, so that a cut shape stays
   closed and the renderer needs no polygon clipping for fills. Section 01 clips the chord polygon to the window
   with Sutherland-Hodgman.
4. **A straight piece is halved, not clipped.** Section 01 clips a line to the window with Liang-Barsky, which
   needs a division of long numbers; halving needs an addition and a shift, and costs the other half of the
   "ten halvings per level".
5. **The window and the pieces are relative to the window's centre**, not to the render origin `R`; the two
   differ by a small offset the caller adds.

## What the design should change

1. Section 06, "Outline algorithm": replace the single path with the union of same-way pieces, or say how the
   single path avoids holes; note the segment count (Finding 1).
2. Section 01, "The cut": the cut of one curve is resumable; the budget is spent per halving, not per object
   (Finding 2). Replace the cost estimates with these figures, native and wasm32.
3. Section 01, "Float rules" and its unverified list: FMA contraction was not seen on x86-64 with this toolchain;
   the outline golden file is the standing check. Add `log2` to the banned list (the camera's display value
   used it; it is now marked as outside shared geometry).
4. Sections 01 and 07: the first wasm32 figures (Finding 4).
5. Section 06, "Stored format" and the tests: the pressure table as built, and what the corpus is.

These are recorded here and made in a separate change.

## Kept afterwards

`outline` and `cut` are part of `everink-core` and stay with it. `build_single_path` and its test go when section
06 is corrected.
