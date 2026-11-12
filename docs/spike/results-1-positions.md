# Spike slice 1: positions

Measured 2026-11-12. Code: `crates/everink-core`. Design under test:
[`../design/01-positions.md`](../design/01-positions.md).

## What was built

- **`Axis`:** one axis of a position as an exact integer of steps of 2^-54 of a level unit. A value that fits 128
  bits is held inline with no allocation; a larger one is a big integer (`num-bigint` 0.5.1). The design writes an
  axis as a cell and an offset; here it is the one integer `cell * 2^64 + offset`, with `cell()` and `frac()` as
  views of it. The two are the same number.
- **`Position`, `Delta`, `Rel`:** level changes (finer is a shift, coarser floors), exact difference and sum across
  levels, exact comparison, and the single conversion to a float: rounded once to 53 bits, ties to even, by this
  code and not by the big-integer library.
- **Canonical bytes:** `encode`, `decode` and `check_cap_bytes`, which refuses a position beyond the depth cap from
  its raw bytes before any large number is built.
- **`Camera`:** exact center, zoom held in [0.5, 2048) by changing level, pan, zoom about a point, an exact jump,
  projection to screen pixels. Numbers from outside that are not finite, or too large to be a gesture, are refused
  and change nothing.

Not built in this slice: outline geometry, the cut, groups (`compose`, `localize`), re-levelling, bounded
surfaces, the render origin as a type. The timing run does the render origin's arithmetic by hand.

**The code layout,** since this is the first code: one Cargo workspace at the root, crates under `crates/`,
edition 2024, the toolchain pinned in `rust-toolchain.toml` (1.97.1, also the minimum supported version),
`Cargo.lock` committed, `unsafe` forbidden, every crate `publish = false`.

## Question 1: what does exact position arithmetic cost?

Command: `cargo run --release --example bench_positions`. Machine: Apple M4, 10 cores, 16 GB, macOS 27.2, Rust
1.97.1, native aarch64. Each figure is over 51 runs of 10,000 objects scattered within a few screens of a point
at the given depth. These are one machine's numbers. A slower machine, and WebAssembly (slice 3), will be slower.

Medians:

| Depth (levels) | Bytes per position | Subtract (ns) | Subtract and convert (ns) | Re-base 10,000 objects (ms) | One frame, 10,000 objects (ms) | Encode (ns) | Decode (ns) |
|---|---|---|---|---|---|---|---|
| 0 | 20 | 9 | 14 | 0.13 | 0.004 | 67 | 113 |
| 1 | 22 | 8 | 13 | 0.12 | 0.004 | 65 | 105 |
| 10 | 45 | 59 | 68 | 0.64 | 0.009 | 235 | 120 |
| 100 | 272 | 121 | 131 | 1.30 | 0.004 | 396 | 214 |
| 1,000 | 2,522 | 446 | 453 | 4.53 | 0.004 | 1,625 | 1,061 |

Spread of the two figures that are compared with a per-frame budget, in milliseconds:

| Depth (levels) | Re-base: minimum | median | 95th percentile | Frame: minimum | median | 95th percentile |
|---|---|---|---|---|---|---|
| 0 | 0.127 | 0.132 | 0.133 | 0.0037 | 0.0037 | 0.0039 |
| 1 | 0.123 | 0.124 | 0.125 | 0.0035 | 0.0036 | 0.0037 |
| 10 | 0.623 | 0.638 | 0.930 | 0.0038 | 0.0085 | 0.0087 |
| 100 | 1.281 | 1.299 | 1.316 | 0.0037 | 0.0037 | 0.0037 |
| 1,000 | 4.308 | 4.533 | 4.562 | 0.0041 | 0.0041 | 0.0043 |

"Re-base" is every object's exact offset from the render origin, turned into a float; it runs when the camera has
moved far from the point the cached floats were measured from. "One frame" is what runs every frame after a pan:
one exact subtraction, then per object a subtraction and two multiplies in floats (by the zoom, and by the
object's level scale, which is 1 in this scene). The frame figure at depth 10 varies between runs of the command
(0.003 to 0.009 ms); the others are steady.

For a second, slower data point: the same command on the CI runner (GitHub-hosted x86-64, shared hardware) gave
2.2 ms for the re-base at depth 100 and 9.5 ms at depth 1,000 at the first revision of this slice. A CI runner
is not a benchmark reference, but it shows the room there is.

### Against the design's estimates

| Quantity | Section 01 estimated | Measured (Apple M4) | Verdict |
|---|---|---|---|
| Subtraction, shallow | about 2 ns | 8 to 9 ns | Slower; negligible either way |
| Subtraction at depth 100 | 20 to 40 ns "with a careful in-place implementation" | 121 ns | 3 to 6 times slower. This code is not in place: it clones a position to bring it to the finer level, and allocates the result |
| Re-base, 10,000 objects, shallow | 0.05 ms | 0.13 ms | About 3 times slower; negligible |
| Re-base, 10,000 objects, depth 100 | 0.5 ms | 1.3 ms | About 3 times slower. Under 2 ms here; over 2 ms on the CI runner |
| Re-base, 10,000 objects, depth 1,000 | not estimated | 4.5 ms | Over 2 ms everywhere |
| One frame, 10,000 objects | 0.05 ms | 0.004 ms | About 10 times faster, and the same at every depth |
| Position size | 19 to 21 bytes shallow, 272 at depth 100, 3,094 at the cap | 20, 22, 272; 3,094 at the cap (test) | As designed |

### What this means

- **The central claim holds.** After a re-base, drawing costs the same at depth 1,000 as at depth 0, because the
  per-frame work is floats. This is the property unlimited zoom depends on.
- **The whiteboard case is cheap.** At depths 0 and 1 every number is inline and a re-base of 10,000 objects takes
  0.13 ms.
- **Depth costs more than estimated, by a constant factor.** The inline path ends between depth 1 and depth 10 (a
  position near the origin needs about 10 bits per level, and 128 fit inline). An in-place subtraction, or storing
  a view's objects as small offsets from a shared base, would remove most of the cost; neither was tried. One
  cheap change was: subtracting two big integers by reference took the depth-100 figure from 135 ns to 121 ns.
- **A re-base does not reliably fit in a frame at depth.** Section 01's 2 ms per frame is the budget of the cut
  queue, which a re-base would share. A re-base of 10,000 objects is inside 2 ms on this machine up to depth 100
  and outside it on a slower one, and at 1,000 levels it is outside it everywhere. It happens about once per two
  screens of panning, not every frame, so the remedy is to spread it over frames or make it incremental. Section 01
  should say so.
- **Canonical bytes are slow to encode at depth** (1.6 microseconds at 1,000 levels), because the cell is split off
  and copied. That supports the design's rule that positions travel as differences from a base inside a message.

## Question 2: are positions the same on every machine?

Test: `tests/golden.rs`. It rebuilds 1,988 lines from a fixed seed and compares them byte for byte with
`tests/golden/positions.txt`, which was written on aarch64. On the machine that wrote the file this proves
nothing; the answer comes from running it elsewhere.

What the lines hold:

| Lines | Count | Content |
|---|---|---|
| `r` | 400 | Random positions over the whole depth range: canonical bytes, a floor to a coarser level, a difference as floats (nearly all too large to be a float) |
| `n` | 280 | Pairs close together at depths 0 to 1,000: the inline rounding path |
| `b` | 400 | Differences of 129 to 1,000 bits that stay inside the float range: the big-integer rounding path, 800 finite floats |
| `t` | 900 | Exact 53-bit ties and one step either side of each, on both rounding paths, both signs |
| `e` | 8 | The ends of the range: at and around 2^1000 and 2^-1000 units |

A second test checks that the `b` and `t` lines are all finite floats, so the file cannot quietly go back to being
almost all "too far".

| Target | Result | Where |
|---|---|---|
| aarch64 macOS | passed | local, Apple M4 |
| x86-64 Linux | passed for the first version of the file (680 lines) in [run 43171633451](https://github.com/vchance/everink/actions/runs/43171633451); for this version, see the `test` job of the latest `ci` run on the pull request that added this slice | GitHub's `ubuntu-24.04` |
| wasm32 | not run | slice 3 adds the target |

## A finding about zooming at depth

Zooming in multiplies any error in the camera's center by the zoom factor. The center loses ten bits each time
the camera moves to a coarser level, so a camera that zooms out to the top and back in **about a fixed point on
the screen** does not come back to the same place. The independent review of this slice measured it: a thing that
started in the middle of the screen ends about 6e18 pixels away after a round trip from depth 10, and is off the
scale from depth 100.

This is not a defect in the arithmetic and cannot be fixed by more precision in the camera: the camera's center at
the top of the trip does not contain the information. What does hold, and is tested:

- **An object never moves.** Its stored position is untouched by anything the camera does.
- **A jump lands exactly.** `Camera::set` puts a position in the middle of the screen exactly, at any depth,
  whatever the camera did before.
- **One zoom step keeps the point under the pointer** to within a millionth of a pixel, at depths 0 to 1,000, for
  a pointer anywhere on screen.
- **Following a thing works.** Zooming out to the top and back while each step is aimed at where the thing is now
  keeps it within a thousandth of a pixel of where it started, from depths 1, 10 and 100.

For the design this confirms the place of navigation aids (home, saved places, jump to a person): at depth they
are the only way back to an exact spot. Section 01's test list has "a point drawn at any depth is found at
exactly the same place later"; that is true of the point and of a jump to it, and false of zooming back by hand.

## Tests

`cargo test --workspace`: 22 tests (20 in `tests/positions.rs`, 2 in `tests/golden.rs`).

- **Canonical bytes:** sizes match the design (19, 21, 272 and 3,094 bytes); encode then decode is the identity
  for 5,000 random positions; non-canonical forms, truncated input and trailing bytes are refused; the cap is
  enforced from the header alone, and in bits, not whole bytes.
- **Exact arithmetic, against a second library:** shifts, floor, difference, sum, level changes and comparison
  are checked against `dashu-int` (a test-only dependency), computed from the definition, for 40,000 pairs a few
  levels apart and 600 pairs up to 2,048 levels apart. The design asked for a million pairs; this is fewer.
- **The inline form at its edges:** values at the ends of the 128-bit range stay canonical, so equality and
  ordering agree.
- **Float conversion:** 40,000 random differences are checked against exact arithmetic to be within half a unit
  in the last place, with ties to even. One in four is built as an exact 53-bit tie or one step off one (the test
  asserts that more than 4,000 of each were generated), and more than half take the big-integer path. A value
  inside the range that came back as zero or as "too far" fails. The ends of the range have their own cases:
  2^1000 units is "too far", and that is decided after rounding, so one step under 2^1000 is too; 2^-1000 is kept
  and anything smaller is zero.
- **Camera:** as listed under the finding above, plus: the zoom stays in range and a level change needs a factor
  of four to reverse; a level change keeps the center (exactly going finer, to within 2^10 steps going coarser)
  and moves nothing on screen; NaN, infinity, zero or negative zoom, a pan beyond 1e12 pixels and a zoom factor
  beyond 1,024 are refused and leave the camera unchanged.

Two tests were checked to be able to fail in the first version of this slice: with the tie rule removed the float
test failed, and with the floor rule removed the shift test failed.

## Where the code departs from the design

1. **An axis is one integer** (128 bits inline), not a 64-bit cell and a 64-bit offset. Same information.
2. **The public API exposes `num_bigint::BigInt`** (`Axis::from_big`, `from_cell_frac`, `to_big`, `cell`). Section
   01 asks for a thin wrapper so the library can be swapped, and the timings above are the reason it might be.
   Before slice 2 builds on this crate, those four should take and return a type of this crate.
3. **The reference tests run 40,600 pairs,** not the million the design names.
4. **A level difference above 2^20 levels panics** (`MAX_SHIFT_LEVELS`) instead of allocating without bound. Bytes
   from the network cannot reach it: `check_cap_bytes` refuses any level beyond the cap first.

## What the design should change

1. Section 01, "Cost": replace the estimates with these figures; say a re-base is spread over frames; say the
   2 ms budget is shared by the cut queue and the re-base.
2. Section 01, "How it is tested" and `design.md` › What proves it: restate "found at exactly the same place" as
   the four properties under the finding above.
3. Section 01, "Positions": describe the inline form as one 128-bit integer.
4. Section 03: delta-encoding positions against a base is needed for speed as well as for size.

These are recorded here and made in a separate change.
