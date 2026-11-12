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
- **`Camera`:** exact center, zoom held in [0.5, 2048) by changing level, pan, zoom about a point, projection to
  screen pixels.

Not built in this slice: outline geometry, the cut, groups (`compose`, `localize`), re-levelling, bounded
surfaces, the render origin as a type. The timing run does the render origin's arithmetic by hand.

## Question 1: what does exact position arithmetic cost?

Command: `cargo run --release --example bench_positions`. Machine: Apple M4, 10 cores, 16 GB, macOS 27.2, Rust
1.97.1. Each figure is the median of 31 runs over 10,000 objects scattered within a few screens of a point at
the given depth. Two runs of the command agreed to within about 10%; the table is the second.

| Depth (levels) | Bytes per position | Subtract (ns) | Subtract and convert (ns) | Re-base 10,000 objects (ms) | One frame, 10,000 objects (ms) | Encode (ns) | Decode (ns) |
|---|---|---|---|---|---|---|---|
| 0 | 20 | 11 | 17 | 0.15 | 0.004 | 77 | 113 |
| 1 | 22 | 8 | 14 | 0.14 | 0.003 | 67 | 102 |
| 10 | 45 | 78 | 82 | 0.82 | 0.003 | 226 | 114 |
| 100 | 272 | 135 | 138 | 1.42 | 0.003 | 391 | 190 |
| 1,000 | 2,522 | 498 | 503 | 5.04 | 0.003 | 1,626 | 1,085 |

"Re-base" is every object's exact offset from the render origin, turned into a float; it runs when the camera has
moved far from the point the cached floats were measured from. "One frame" is what runs every frame after a pan:
one exact subtraction, then a few float operations per object.

### Against the design's estimates

| Quantity | Section 01 estimated | Measured | Verdict |
|---|---|---|---|
| Subtraction, shallow | about 2 ns | 8 to 11 ns | Slower, and it does not matter: see re-base |
| Subtraction at depth 100 | 20 to 40 ns "with a careful in-place implementation" | 135 ns | 3 to 4 times slower. This implementation is not in place: it allocates a big integer per subtraction |
| Re-base, 10,000 objects, shallow | 0.05 ms | 0.15 ms | 3 times slower; still negligible |
| Re-base, 10,000 objects, depth 100 | 0.5 ms | 1.4 ms | 3 times slower; inside the 2 ms per frame budget |
| Re-base, 10,000 objects, depth 1,000 | not estimated | 5.0 ms | Over the 2 ms budget |
| One frame, 10,000 objects | 0.05 ms | 0.003 ms | 15 times faster, and the same at every depth |
| Position size | 19 to 21 bytes shallow, 272 at depth 100, 3,094 at the cap | 20, 22, 272; 3,094 at the cap (test) | As designed |

### What this means

- **The central claim holds.** After a re-base, drawing costs the same at depth 1,000 as at depth 0, because the
  per-frame work is floats. This is the property unlimited zoom depends on.
- **The whiteboard case is cheap.** At depths 0 and 1 every number is inline and a re-base of 10,000 objects takes
  0.15 ms.
- **Depth costs more than estimated, by a constant factor.** The inline path ends between depth 1 and depth 10
  (a position near the origin needs about 10 bits per level and 128 fit inline). Past it, each subtraction
  allocates. An in-place subtraction, or storing a view's objects as small offsets from a shared base, would
  remove most of that; neither was tried.
- **A re-base at extreme depth does not fit one frame.** At 1,000 levels it takes 5 ms for 10,000 objects. A
  re-base happens about once per two screens of panning, not every frame, so the remedy is to spread it over a few
  frames or to make it incremental. Section 01 should say so.
- **Canonical bytes are slow to encode at depth** (1.6 microseconds at 1,000 levels), because the cell is split off
  and copied. That supports the design's rule that positions travel as differences from a base inside a message.

## Question 2: are positions the same on every machine?

Test: `tests/golden.rs`. It rebuilds 680 lines from a fixed seed (random positions over the whole depth range
with their bytes, a level change each way and a difference as float bits; and pairs close together at depths 0 to
1,000) and compares them byte for byte with `tests/golden/positions.txt`, which was written on aarch64.

| Target | Result | Where |
|---|---|---|
| aarch64 macOS | passed | local, this revision |
| x86-64 Linux | see the CI run of the change that added this file | `test` job of `ci` |
| wasm32 | not run | slice 3 adds the target |

## Tests

`cargo test --workspace`: 14 tests.

- Canonical bytes: sizes match the design (19, 21, 272 and 3,094 bytes); encode then decode is the identity for
  5,000 random positions; non-canonical forms, truncated input, trailing bytes and anything beyond the cap are
  refused, the last from the header alone.
- Exact arithmetic: 20,000 random shifts agree with a big-integer reference and floor correctly; cell and offset
  are the high and low part; difference and sum are exact across levels; a level change keeps the point.
- Float conversion: 40,000 random differences, one in eight an exact tie or one step off a tie, are checked
  against exact arithmetic to be within half a unit in the last place with ties to even.
- Camera: the zoom stays in range and a level change needs a factor of four to reverse; a level change moves
  nothing on screen (bit-identical going finer, under 1e-9 pixel going coarser); a point drawn at depths 1, 10, 100
  and 1,000 is found at exactly the same place after zooming out to the top and back.

Two of these were checked to be able to fail: with the tie rule removed the float test fails, and with the floor
rule removed the shift test fails.

## What the design should change

1. Section 01, "Cost": replace the estimates with these figures, and say that a re-base is spread over frames when
   it would exceed the budget.
2. Section 01, "Positions": the inline form is one 128-bit integer, not a 64-bit cell and a 64-bit offset. The
   information is the same; the description should match the code.
3. Section 03: delta-encoding positions against a base is needed for speed as well as for size.

These are recorded here and made in a separate change.
