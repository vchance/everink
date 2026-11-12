//! The cut: a curve far larger than the window is cut down to the part that crosses it.

// The checks here compare floats with ordinary float helpers; none of it is shared geometry.
#![allow(clippy::disallowed_methods)]

mod common;

use common::{Rng, axis_big, big_of};
use everink_core::Axis;
use everink_core::cut::{BigPt, CutStats, Piece, cut_quad, place};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

/// The window's half-side: 2,048 camera-level units, in steps of 2^-40.
const HALF: i64 = 2048 << 40;
const STEPS: f64 = 1_099_511_627_776.0;
const HALF_UNITS: f64 = 2048.0;

fn pt(x: BigInt, y: BigInt) -> BigPt {
    BigPt {
        x: axis_big(x),
        y: axis_big(y),
    }
}

/// A curve from `levels` levels above the window that passes through the point `w` (inside the window) at its
/// middle: its other two control points are random and about 4,096 units times 1024 per level away, and the
/// first is chosen so that `(p0 + 2 p1 + p2) / 4 = w`.
fn curve_through(rng: &mut Rng, levels: u64) -> ([BigInt; 6], [BigInt; 2]) {
    let bits = 10 * levels + 40 + 12;
    let w = [
        BigInt::from(rng.next() as i64 >> 14),
        BigInt::from(rng.next() as i64 >> 14),
    ]; // within 2^49 steps, inside the window
    let (p1x, p1y, p2x, p2y) = (rng.big(bits), rng.big(bits), rng.big(bits), rng.big(bits));
    let p0x = &w[0] * 4 - &p1x * 2 - &p2x;
    let p0y = &w[1] * 4 - &p1y * 2 - &p2y;
    ([p0x, p0y, p1x, p1y, p2x, p2y], w)
}

fn cut(c: &[BigInt; 6]) -> (Vec<Piece>, CutStats) {
    let mut out = Vec::new();
    let stats = cut_quad(
        [
            pt(c[0].clone(), c[1].clone()),
            pt(c[2].clone(), c[3].clone()),
            pt(c[4].clone(), c[5].clone()),
        ],
        HALF,
        &mut out,
    );
    (out, stats)
}

/// The distance from a point to a line segment.
fn distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ex, ey) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    (ex * ex + ey * ey).sqrt()
}

/// The pieces as one run of straight lines, each curve flattened into 16.
fn polyline(pieces: &[Piece], with_far: bool) -> Vec<([f64; 2], [f64; 2])> {
    flattened(pieces, with_far, 16)
}

fn flattened(pieces: &[Piece], with_far: bool, parts: u32) -> Vec<([f64; 2], [f64; 2])> {
    let mut lines = Vec::new();
    for piece in pieces {
        match *piece {
            Piece::Line(a, b) => lines.push((a, b)),
            Piece::Far(a, b) => {
                if with_far {
                    lines.push((a, b));
                }
            }
            Piece::Quad(a, c, b) => {
                let at = |t: f64| {
                    let u = 1.0 - t;
                    [
                        u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0],
                        u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1],
                    ]
                };
                for k in 0..parts {
                    let (t0, t1) = (
                        f64::from(k) / f64::from(parts),
                        f64::from(k + 1) / f64::from(parts),
                    );
                    lines.push((at(t0), at(t1)));
                }
            }
        }
    }
    lines
}

fn assert_chain(pieces: &[Piece], what: &str) {
    for pair in pieces.windows(2) {
        assert_eq!(
            pair[0].end(),
            pair[1].start(),
            "{what}: a gap between pieces"
        );
    }
}

#[test]
fn a_curve_through_the_window_is_found_at_every_depth() {
    let mut rng = Rng(41);
    for levels in [1u64, 2, 3, 5, 10, 50, 100, 200, 500] {
        for case in 0..20 {
            let (c, w) = curve_through(&mut rng, levels);
            let (pieces, stats) = cut(&c);
            let what = format!("{levels} levels, case {case}");
            assert_chain(&pieces, &what);
            let near = polyline(&pieces, false);
            assert!(
                !near.is_empty(),
                "{what}: nothing of the curve is near the window"
            );
            // The curve passes through `w`, so the cut pieces must too, to well under a millionth of a unit:
            // each halving may round by one step of 2^-40 of a unit.
            let w_units = [
                w[0].to_f64().unwrap() / STEPS,
                w[1].to_f64().unwrap() / STEPS,
            ];
            let closest = near
                .iter()
                .map(|(a, b)| distance(w_units, *a, *b))
                .fold(f64::INFINITY, f64::min);
            assert!(
                closest < 1e-6,
                "{what}: the pieces miss the curve's point by {closest}"
            );
            // From five levels up, the curve is straight across the window to far below a step (at three it
            // still bends by a ten-thousandth of a unit over the nine windows a near piece may reach), and its
            // direction there is `p2 - p0`. Every near point must lie on that line through `w`.
            if levels >= 5 {
                let (tx, ty) = (&c[4] - &c[0], &c[5] - &c[1]);
                let shift = tx.bits().max(ty.bits()).saturating_sub(60);
                let (tx, ty) = (
                    (tx >> shift).to_f64().unwrap(),
                    (ty >> shift).to_f64().unwrap(),
                );
                let len = (tx * tx + ty * ty).sqrt();
                for (a, b) in &near {
                    for p in [a, b] {
                        let off = ((p[0] - w_units[0]) * ty - (p[1] - w_units[1]) * tx) / len;
                        assert!(
                            off.abs() < 1e-5,
                            "{what}: a near point is {off} units off the curve"
                        );
                    }
                }
            }
            // The work is about ten halvings per level, of which the shortcut makes about half cheap.
            assert!(
                u64::from(stats.depth) <= 10 * levels + 30,
                "{what}: halved {} deep",
                stats.depth
            );
            // Nothing visible is outside four windows; nothing at all is outside the far box.
            for (a, b) in polyline(&pieces, true) {
                for p in [a, b] {
                    assert!(p[0].abs() <= 16.0 * HALF_UNITS && p[1].abs() <= 16.0 * HALF_UNITS);
                }
            }
        }
    }
}

#[test]
fn a_small_curve_is_cut_to_what_floats_give() {
    // Curves a few windows across, where plain floats can follow the true curve: every point of the true curve
    // inside the window is on the cut pieces, and every near piece is on the true curve.
    let mut rng = Rng(42);
    for case in 0..200 {
        let p: Vec<i64> = (0..6).map(|_| rng.next() as i64 >> 9).collect(); // within 2^54 steps: 8 windows
        let c: [BigInt; 6] = std::array::from_fn(|i| BigInt::from(p[i]));
        let (pieces, _) = cut(&c);
        assert_chain(&pieces, "small");
        // A piece may be four windows across and strongly bent, so it is flattened finely for this check.
        let near = flattened(&pieces, false, 512);
        let f: Vec<f64> = p.iter().map(|v| *v as f64 / STEPS).collect();
        let at = |t: f64| {
            let u = 1.0 - t;
            [
                u * u * f[0] + 2.0 * u * t * f[2] + t * t * f[4],
                u * u * f[1] + 2.0 * u * t * f[3] + t * t * f[5],
            ]
        };
        let truth: Vec<[f64; 2]> = (0..=4000).map(|k| at(f64::from(k) / 4000.0)).collect();
        let mut inside = 0;
        for q in &truth {
            if q[0].abs() < HALF_UNITS && q[1].abs() < HALF_UNITS {
                inside += 1;
                let d = near
                    .iter()
                    .map(|(a, b)| distance(*q, *a, *b))
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    d < 0.05,
                    "case {case}: a point of the curve is {d} units off the pieces"
                );
            }
        }
        if inside == 0 {
            // The curve's box may still touch the window; then the pieces are near but outside it.
            for (a, b) in &near {
                for q in [a, b] {
                    assert!(q[0].abs() <= 9.0 * HALF_UNITS && q[1].abs() <= 9.0 * HALF_UNITS);
                }
            }
        }
        for (a, _) in &near {
            let d = truth
                .windows(2)
                .map(|w| distance(*a, w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(d < 0.05, "case {case}: a piece is {d} units off the curve");
        }
    }
}

#[test]
fn a_curve_that_misses_the_window_leaves_only_its_chord() {
    let mut rng = Rng(43);
    for levels in [1u64, 10, 100, 1000] {
        for _ in 0..20 {
            // Every control point far to one side of the window.
            let bits = 10 * levels + 52;
            let far = BigInt::from(1) << (bits + 2);
            let c: [BigInt; 6] = std::array::from_fn(|i| {
                if i % 2 == 0 {
                    &far + rng.big(bits)
                } else {
                    rng.big(bits)
                }
            });
            let (pieces, stats) = cut(&c);
            assert_eq!(stats.halvings, 0, "nothing to halve");
            assert!(pieces.iter().all(|p| matches!(p, Piece::Far(..))));
            assert!(pieces.len() <= 1);
        }
    }
}

/// How many times the pieces wind round the centre of the window.
fn winding(pieces: &[Piece]) -> i32 {
    let mut w = 0;
    for (a, b) in polyline(pieces, true) {
        let side = a[0] * b[1] - a[1] * b[0];
        if a[1] <= 0.0 {
            if b[1] > 0.0 && side > 0.0 {
                w += 1;
            }
        } else if b[1] <= 0.0 && side < 0.0 {
            w -= 1;
        }
    }
    w
}

#[test]
fn a_shape_cut_to_the_window_still_goes_round_it_or_not() {
    // A triangle hundreds of levels larger than the window. With the window inside it, the cut pieces (the far
    // chords, pulled onto the far box) must still go once round the window's centre; with the window outside,
    // not at all. This is what lets the renderer fill the window, or leave it empty, from the pieces alone.
    let mut rng = Rng(44);
    for levels in [1u64, 10, 100, 500] {
        let big = BigInt::from(1) << (10 * levels + 52);
        let corner = |x: i32, y: i32, rng: &mut Rng| {
            let jitter = 10 * levels + 40;
            [&big * x + rng.big(jitter), &big * y + rng.big(jitter)]
        };
        for (corners, want) in [
            ([(-3, -2), (4, -1), (0, 5)], 1),  // round the origin, anticlockwise
            ([(0, 5), (4, -1), (-3, -2)], -1), // the same, clockwise
            ([(2, 1), (9, 2), (5, 8)], 0),     // off to one side
            ([(-9, -9), (-2, -8), (-8, -1)], 0),
        ] {
            let pts: Vec<[BigInt; 2]> = corners
                .iter()
                .map(|&(x, y)| corner(x, y, &mut rng))
                .collect();
            let mut pieces = Vec::new();
            for i in 0..3 {
                let (a, b) = (&pts[i], &pts[(i + 1) % 3]);
                // A straight side, as a curve whose control point is its middle.
                let m = [(&a[0] + &b[0]) >> 1u32, (&a[1] + &b[1]) >> 1u32];
                cut_quad(
                    [
                        pt(a[0].clone(), a[1].clone()),
                        pt(m[0].clone(), m[1].clone()),
                        pt(b[0].clone(), b[1].clone()),
                    ],
                    HALF,
                    &mut pieces,
                );
            }
            // Off to one side, every chord may be pulled onto one corner of the far box, and nothing is left.
            if pieces.is_empty() {
                assert_eq!(
                    want, 0,
                    "{levels} levels: the shape round the window vanished"
                );
                continue;
            }
            assert_eq!(
                pieces.last().unwrap().end(),
                pieces[0].start(),
                "{levels} levels: the cut shape is not closed"
            );
            assert_chain(&pieces, "triangle");
            assert_eq!(
                winding(&pieces),
                want,
                "{levels} levels, corners {corners:?}"
            );
        }
    }
}

#[test]
fn a_shape_point_is_placed_on_the_cameras_level() {
    // A point of a shape 3 levels above the camera, and a window centre given on the camera's level.
    let centre = BigPt {
        x: Axis::from_i128(5),
        y: Axis::from_i128(-7),
    };
    let p = place(3, -2, 3, &centre);
    assert_eq!(big_of(&p.x), (BigInt::from(3) << 30u32) - 5);
    assert_eq!(big_of(&p.y), (BigInt::from(-2) << 30u32) + 7);
    // A thousand levels up, the numbers are ten thousand bits long and still exact.
    let p = place(i64::MAX, i64::MIN, 1000, &centre);
    assert_eq!(big_of(&p.x), (BigInt::from(i64::MAX) << 10_000u32) - 5);
    assert_eq!(big_of(&p.y), (BigInt::from(i64::MIN) << 10_000u32) + 7);
}
