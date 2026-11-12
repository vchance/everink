//! Stroke outlines: the same on every machine, closed, and covering the stroke they were built from.
//!
//! `tests/golden/outlines.txt` holds one line per stroke of a fixed corpus: for each of the two builders, the
//! number of segments and a hash of every coordinate of the outline. It was written on one machine (aarch64)
//! and must pass byte for byte on the others (x86-64, which has a fused multiply-add, and wasm32). Rewrite it
//! only when an outline algorithm is meant to change: `UPDATE_GOLDEN=1 cargo test -p everink-core --test outlines`.

mod common;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use common::Rng;
use everink_core::outline::{
    InkPoint, LPt, Outline, OutlineError, STYLE_VERSION, Seg, build, build_single_path,
};

const STROKES: usize = 1_000;

/// A stroke of the corpus: a random walk as a hand makes, with the cases an outline has to survive mixed in:
/// sharp turns, doubling back, a pause on one point, no pressure, a single dot, the far corners of the lattice.
/// The pressure changes at random from point to point, which is rougher than a real pen.
fn stroke(rng: &mut Rng) -> (Vec<InkPoint>, i32) {
    let n = match rng.below(20) {
        0 => 1,
        1 => 2,
        _ => 3 + rng.below(300) as usize,
    };
    // A width from a hair to a marker, in 16.16 object units: an eighth of a unit to five units.
    let width = (1 + rng.below(40) as i32) << 13;
    let no_pressure = rng.below(8) == 0;
    // Most strokes are near the middle of the lattice; some start near its edge.
    let (mut x, mut y) = if rng.below(10) == 0 {
        (i32::MAX - 20_000_000, i32::MIN + 20_000_000)
    } else {
        (rng.next() as i32 >> 8, rng.next() as i32 >> 8)
    };
    let (mut dx, mut dy) = (rng.below(4001) as i32 - 2000, rng.below(4001) as i32 - 2000);
    let mut points = Vec::with_capacity(n);
    for _ in 0..n {
        match rng.below(40) {
            0 => (dx, dy) = (-dx, -dy),                   // straight back
            1 => (dx, dy) = (-dy, dx),                    // a right angle
            2 => (dx, dy) = (-dx + dy / 8, -dy - dx / 8), // a hairpin
            3 => (dx, dy) = (0, 0),                       // a pause
            4 => (dx, dy) = (rng.below(4001) as i32 - 2000, rng.below(4001) as i32 - 2000),
            _ => {
                dx += rng.below(201) as i32 - 100;
                dy += rng.below(201) as i32 - 100;
            }
        }
        x = x.saturating_add(dx);
        y = y.saturating_add(dy);
        points.push(InkPoint {
            x,
            y,
            pressure: if no_pressure {
                0
            } else {
                1 + rng.below(255) as u8
            },
        });
    }
    (points, width)
}

fn corpus() -> Vec<(Vec<InkPoint>, i32)> {
    let mut rng = Rng(0x0717);
    (0..STROKES).map(|_| stroke(&mut rng)).collect()
}

/// FNV-1a over every coordinate, in order, little-endian.
fn hash(outline: &Outline) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |p: LPt| {
        for b in p.x.to_le_bytes().into_iter().chain(p.y.to_le_bytes()) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for seg in &outline.segs {
        match seg {
            Seg::Line(a, b) => {
                eat(*a);
                eat(*b);
            }
            Seg::Quad(a, b, c) => {
                eat(*a);
                eat(*b);
                eat(*c);
            }
        }
    }
    h
}

#[test]
fn outlines_match_the_golden_file_byte_for_byte() {
    let mut built = String::new();
    writeln!(
        built,
        "# everink-core stroke outlines, style version {STYLE_VERSION}"
    )
    .unwrap();
    writeln!(
        built,
        "# o <stroke> <points> <segments> <hash> <single-path segments> <single-path hash>"
    )
    .unwrap();
    for (i, (points, width)) in corpus().iter().enumerate() {
        let outline = build(points, *width).unwrap();
        let single = build_single_path(points, *width).unwrap();
        writeln!(
            built,
            "o {i} {} {} {:016x} {} {:016x}",
            points.len(),
            outline.segs.len(),
            hash(&outline),
            single.segs.len(),
            hash(&single)
        )
        .unwrap();
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/outlines.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        assert!(
            std::env::var_os("CI").is_none(),
            "the golden file is never rewritten in CI: a difference between machines is the bug"
        );
        std::fs::write(&path, &built).unwrap();
        return;
    }
    let golden =
        std::fs::read_to_string(&path).expect("tests/golden/outlines.txt is in the repository");
    if golden != built {
        let first = golden
            .lines()
            .zip(built.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "outlines differ from the golden file, first at line {}:\n  golden: {:?}\n  built:  {:?}",
            first + 1,
            golden.lines().nth(first),
            built.lines().nth(first)
        );
    }
}

/// The closed paths of an outline, each as its polygon of start and control points.
fn paths(outline: &Outline) -> Vec<Vec<LPt>> {
    let mut all: Vec<Vec<LPt>> = Vec::new();
    let mut end: Option<LPt> = None;
    for seg in &outline.segs {
        if end != Some(seg.start()) {
            all.push(Vec::new());
        }
        let path = all.last_mut().unwrap();
        path.push(seg.start());
        if let Seg::Quad(_, c, _) = seg {
            path.push(*c);
        }
        end = Some(seg.end());
    }
    all
}

/// The winding number of the outline's polygons round a lattice point: whole-number arithmetic only.
fn winding(polygons: &[Vec<LPt>], p: LPt) -> i32 {
    let mut w = 0;
    for pts in polygons {
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let side = (i128::from(b.x) - i128::from(a.x)) * (i128::from(p.y) - i128::from(a.y))
                - (i128::from(b.y) - i128::from(a.y)) * (i128::from(p.x) - i128::from(a.x));
            if a.y <= p.y {
                if b.y > p.y && side > 0 {
                    w += 1;
                }
            } else if b.y <= p.y && side < 0 {
                w -= 1;
            }
        }
    }
    w
}

fn to_outline(v: i32) -> i64 {
    i64::from(v) << 24 // 2^-16 of a unit to 2^-40
}

/// Checks that every path of the outline is closed and that nothing strays further from the points than the
/// full width (an edge sits at most half a width out).
fn closed_and_in_reach(i: usize, points: &[InkPoint], width: i32, outline: &Outline) {
    assert!(!outline.segs.is_empty(), "stroke {i}");
    let mut start = outline.segs[0].start();
    let mut end = start;
    for seg in &outline.segs {
        if seg.start() != end {
            assert_eq!(end, start, "stroke {i}: a path is not closed");
            start = seg.start();
        }
        end = seg.end();
    }
    assert_eq!(end, start, "stroke {i}: the last path is not closed");
    let reach = i64::from(width) << 24;
    let (min_x, max_x) = points.iter().fold((i64::MAX, i64::MIN), |(lo, hi), p| {
        (lo.min(to_outline(p.x)), hi.max(to_outline(p.x)))
    });
    let (min_y, max_y) = points.iter().fold((i64::MAX, i64::MIN), |(lo, hi), p| {
        (lo.min(to_outline(p.y)), hi.max(to_outline(p.y)))
    });
    for p in paths(outline).iter().flatten() {
        assert!(
            p.x >= min_x - reach && p.x <= max_x + reach,
            "stroke {i}: x out of reach"
        );
        assert!(
            p.y >= min_y - reach && p.y <= max_y + reach,
            "stroke {i}: y out of reach"
        );
    }
}

#[test]
fn an_outline_is_closed_and_covers_its_stroke() {
    let mut checked = 0;
    for (i, (points, width)) in corpus().iter().enumerate() {
        let outline = build(points, *width).unwrap();
        closed_and_in_reach(i, points, *width, &outline);
        let polygons = paths(&outline);
        // Every point of the stroke, and the middle of every piece between two points, is inside the outline,
        // and every path goes the same way round, so the count is never negative.
        // Every eighth point is checked, and the first and last; the check is a walk round the whole outline.
        let last = points.len() - 1;
        for (k, p) in points
            .iter()
            .enumerate()
            .filter(|(k, _)| k % 8 == 0 || *k == last)
        {
            let at = LPt {
                x: to_outline(p.x),
                y: to_outline(p.y),
            };
            assert!(
                winding(&polygons, at) > 0,
                "stroke {i}: point {k} is outside its outline"
            );
            if let Some(q) = points.get(k + 1) {
                let mid = LPt {
                    x: (to_outline(p.x) + to_outline(q.x)) / 2,
                    y: (to_outline(p.y) + to_outline(q.y)) / 2,
                };
                assert!(
                    winding(&polygons, mid) > 0,
                    "stroke {i}: the middle after point {k} is outside its outline"
                );
            }
            checked += 2;
        }
    }
    assert!(checked > 30_000);
}

#[test]
fn the_single_path_of_the_design_leaves_holes_over_the_stroke() {
    // Section 06 first specified one closed path: up one side, round the end, down the other. Where a stroke
    // doubles back or curls tightly under a wide brush, the two sides cross, and the non-zero rule leaves a
    // hole over the stroke's own centre line. This records how often, on the same corpus `build` covers fully.
    let (mut checked, mut outside) = (0, 0);
    let mut strokes_with_a_hole = BTreeSet::new();
    for (i, (points, width)) in corpus().iter().enumerate() {
        let outline = build_single_path(points, *width).unwrap();
        closed_and_in_reach(i, points, *width, &outline);
        let polygons = paths(&outline);
        assert_eq!(polygons.len(), 1, "stroke {i}: one path");
        for p in points {
            let at = LPt {
                x: to_outline(p.x),
                y: to_outline(p.y),
            };
            checked += 1;
            if winding(&polygons, at) == 0 {
                outside += 1;
                strokes_with_a_hole.insert(i);
            }
        }
    }
    println!(
        "single path: {outside} of {checked} stroke points are outside their outline, in {} of {STROKES} strokes",
        strokes_with_a_hole.len()
    );
    // A record of a limit: if this ever reaches zero the single path has been repaired, and the results file
    // of slice 3a should be read again.
    assert!(outside * 100 > checked * 3, "{outside} of {checked}");
    assert!(strokes_with_a_hole.len() > 200);
}

#[test]
fn a_dot_is_a_circle_and_bad_input_is_refused() {
    let dot = [InkPoint {
        x: 1000,
        y: -2000,
        pressure: 255,
    }];
    for outline in [
        build(&dot, 4 << 16).unwrap(),
        build_single_path(&dot, 4 << 16).unwrap(),
    ] {
        assert_eq!(outline.segs.len(), 16);
        // Every corner of a dot is one radius from its centre: 2 units, to within a lattice step.
        let (cx, cy) = (1000i64 << 24, -2000i64 << 24);
        for seg in &outline.segs {
            let Seg::Quad(_, c, _) = seg else {
                panic!("a line in a dot");
            };
            let (dx, dy) = (i128::from(c.x - cx), i128::from(c.y - cy));
            let r2 = dx * dx + dy * dy;
            let want = 2i128 << 40;
            assert!(
                (r2 - want * want).abs() < want * 8,
                "a corner is off the circle"
            );
        }
        // The same point many times is still a dot, and the heaviest press sets its size.
        let mut pause = vec![dot[0]; 50];
        pause[10].pressure = 1;
        assert_eq!(build(&pause, 4 << 16).unwrap().segs.len(), 16);
    }
    for f in [build, build_single_path] {
        assert_eq!(f(&[], 1 << 16), Err(OutlineError::Empty));
        assert_eq!(f(&dot, 0), Err(OutlineError::BadWidth));
        assert_eq!(f(&dot, -5), Err(OutlineError::BadWidth));
    }
}

#[test]
fn pressure_sets_the_width() {
    // A straight stroke along x: the outline's reach in y is half the width times the share for the pressure.
    let line = |pressure: u8| -> i64 {
        let points: Vec<InkPoint> = (0..20)
            .map(|i| InkPoint {
                x: i * 5000,
                y: 0,
                pressure,
            })
            .collect();
        let outline = build(&points, 8 << 16).unwrap();
        paths(&outline).iter().flatten().map(|p| p.y).max().unwrap()
    };
    let full = 4i64 << 40; // half of a width of 8 units
    assert_eq!(line(255), full);
    assert_eq!(
        line(0),
        full,
        "no pressure from the device is the full width"
    );
    assert_eq!(line(1), full / 4, "the lightest touch is a quarter");
    let reaches: Vec<i64> = (1..=255).map(line).collect();
    assert!(
        reaches.windows(2).all(|w| w[0] <= w[1]),
        "more pressure is never thinner"
    );
}
