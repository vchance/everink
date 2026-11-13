//! Stroke outlines: the same on every machine, closed, and covering the stroke they were built from.
//!
//! `tests/golden/outlines.txt` holds one line per stroke of a fixed corpus: for each of the two builders, the
//! number of segments and a hash of every coordinate of the outline. It was written on one machine (aarch64)
//! and must pass byte for byte on the others: x86-64 as it is built by default and with its fused multiply-add
//! switched on, and wasm32. aarch64 always has a fused multiply-add, so the file itself was written where the
//! compiler could have used one. Rewrite it
//! only when an outline algorithm is meant to change: `UPDATE_GOLDEN=1 cargo test -p everink-core --test outlines`.
//! `tests/golden/outlines_runs.txt` is the same for the runs of slice 3d, a trial form, in a file of its own so
//! that the first is not rewritten for it.

mod common;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use common::Rng;
use everink_core::outline::{
    InkPoint, LPt, Outline, OutlineError, STYLE_VERSION, Seg, build, build_runs, build_runs_within,
    build_single_path,
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

/// The runs as they are tried: cut at a quarter turn, cut at an eighth, and cut at an eighth with the sides
/// smoothed.
const RUNS: [(&str, f64, bool); 3] = [
    ("cut at a quarter turn", 0.0, false),
    ("cut at an eighth", std::f64::consts::FRAC_1_SQRT_2, false),
    (
        "cut at an eighth, sides smoothed",
        std::f64::consts::FRAC_1_SQRT_2,
        true,
    ),
];

#[test]
fn runs_match_their_golden_file_byte_for_byte() {
    let mut built = String::new();
    writeln!(
        built,
        "# everink-core stroke outlines as runs (spike slice 3d), style version {STYLE_VERSION}"
    )
    .unwrap();
    writeln!(
        built,
        "# r <stroke> <points>, then <segments> <hash> for the runs {}",
        RUNS.map(|r| r.0).join("; ")
    )
    .unwrap();
    for (i, (points, width)) in corpus().iter().enumerate() {
        write!(built, "r {i} {}", points.len()).unwrap();
        for (_, limit, smooth) in RUNS {
            let outline = build_runs_within(points, *width, limit, smooth).unwrap();
            write!(built, " {} {:016x}", outline.segs.len(), hash(&outline)).unwrap();
        }
        writeln!(built).unwrap();
        // `build_runs` is the first of them.
        assert_eq!(
            build_runs(points, *width).unwrap().segs,
            build_runs_within(points, *width, RUNS[0].1, RUNS[0].2)
                .unwrap()
                .segs
        );
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/outlines_runs.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        assert!(
            std::env::var_os("CI").is_none(),
            "the golden file is never rewritten in CI: a difference between machines is the bug"
        );
        std::fs::write(&path, &built).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("tests/golden/outlines_runs.txt is in the repository");
    if golden != built {
        let first = golden
            .lines()
            .zip(built.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "runs differ from the golden file, first at line {}:\n  golden: {:?}\n  built:  {:?}",
            first + 1,
            golden.lines().nth(first),
            built.lines().nth(first)
        );
    }
}

/// Twice the area a polygon goes round, anticlockwise counted as positive. Whole numbers only.
fn twice_area(polygon: &[LPt]) -> i128 {
    (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
        })
        .sum()
}

/// A polygon with the box it lies in: lowest x and y, highest x and y.
type Boxed = (Vec<LPt>, [i64; 4]);

/// The closed paths of an outline with every curve followed: each quadratic curve as eight chords between
/// points of the curve itself, in whole numbers. A chord of a curve is on the inner side of it, so a point
/// inside these polygons is inside the outline.
fn fine_paths(outline: &Outline) -> Vec<Boxed> {
    let mut all: Vec<Vec<LPt>> = Vec::new();
    let mut end: Option<LPt> = None;
    for seg in &outline.segs {
        if end != Some(seg.start()) {
            all.push(Vec::new());
        }
        let path = all.last_mut().unwrap();
        path.push(seg.start());
        if let Seg::Quad(a, c, b) = seg {
            for k in 1..8i128 {
                let at = |a: i64, c: i64, b: i64| {
                    let sum = (8 - k) * (8 - k) * i128::from(a)
                        + 2 * k * (8 - k) * i128::from(c)
                        + k * k * i128::from(b);
                    i64::try_from(sum / 64).unwrap()
                };
                path.push(LPt {
                    x: at(a.x, c.x, b.x),
                    y: at(a.y, c.y, b.y),
                });
            }
        }
        end = Some(seg.end());
    }
    all.into_iter()
        .map(|path| {
            let bounds = path
                .iter()
                .fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |b, p| {
                    [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]
                });
            (path, bounds)
        })
        .collect()
}

/// `winding` over the polygons of `fine_paths`. A polygon does not wind round a point outside its box.
fn fine_winding(polygons: &[Boxed], p: LPt) -> i32 {
    polygons
        .iter()
        .filter(|(_, b)| b[0] <= p.x && p.x <= b[2] && b[1] <= p.y && p.y <= b[3])
        .map(|(polygon, _)| winding(std::slice::from_ref(polygon), p))
        .sum()
}

#[test]
fn runs_are_closed_cover_the_stroke_and_are_light() {
    let corpus = corpus();
    let full: usize = corpus
        .iter()
        .map(|(points, width)| build(points, *width).unwrap().segs.len())
        .sum();
    let single: usize = corpus
        .iter()
        .map(|(points, width)| build_single_path(points, *width).unwrap().segs.len())
        .sum();
    for (name, limit, smooth) in RUNS {
        let (mut checked, mut segments, mut paths_made) = (0, 0, 0);
        for (i, (points, width)) in corpus.iter().enumerate() {
            let outline = build_runs_within(points, *width, limit, smooth).unwrap();
            closed_and_in_reach(i, points, *width, &outline);
            segments += outline.segs.len();
            let polygons = fine_paths(&outline);
            paths_made += polygons.len();
            // Every path goes the same way round, so no path takes away what another covers.
            for (polygon, _) in &polygons {
                assert!(twice_area(polygon) > 0, "stroke {i}: a path goes clockwise");
            }
            // The points of the stroke, and the middle of the piece after each, are inside the outline.
            // Every eighth point is checked, and the first and last; unlike the test of `build`, the
            // curves are followed and not read as their control points.
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
                    fine_winding(&polygons, at) > 0,
                    "stroke {i}, runs {name}: point {k} is outside its outline"
                );
                if let Some(q) = points.get(k + 1) {
                    let mid = LPt {
                        x: (to_outline(p.x) + to_outline(q.x)) / 2,
                        y: (to_outline(p.y) + to_outline(q.y)) / 2,
                    };
                    assert!(
                        fine_winding(&polygons, mid) > 0,
                        "stroke {i}, runs {name}: the middle after point {k} is outside its outline"
                    );
                }
                checked += 2;
            }
        }
        println!(
            "runs {name}: {segments} segments in {paths_made} paths for {STROKES} strokes; discs and bands {full}, the single path {single}"
        );
        assert!(checked > 30_000);
        // The corpus is rough: a sharp turn, a doubling back or a pause about every thirteenth point. Even so
        // the runs are well under half of discs and bands.
        assert!(segments * 2 < full, "{segments} against {full}");
    }
}

#[test]
fn runs_handle_the_small_cases() {
    let at = |x: i32, y: i32| InkPoint {
        x,
        y,
        pressure: 128,
    };
    let width = 1 << 16;
    // One point, and a point given twice: a disc.
    for points in [vec![at(0, 0)], vec![at(5, 5), at(5, 5)]] {
        let outline = build_runs(&points, width).unwrap();
        assert_eq!(paths(&outline).len(), 1);
        assert_eq!(
            outline.segs.len(),
            build(&points, width).unwrap().segs.len()
        );
    }
    // Two points: a disc at each end and one four-cornered run between.
    let outline = build_runs(&[at(0, 0), at(100_000, 0)], width).unwrap();
    assert_eq!(paths(&outline).len(), 3);
    assert_eq!(outline.segs.len(), 16 + 4 + 16);
    // A straight line of many points is still two discs and one run, two segments a point and two ends.
    let line: Vec<InkPoint> = (0..50).map(|i| at(i * 10_000, i * 5_000)).collect();
    let outline = build_runs(&line, width).unwrap();
    assert_eq!(paths(&outline).len(), 3);
    assert_eq!(outline.segs.len(), 16 + 2 * 50 + 16);
    // Straight back: the run ends at the turn, under a disc, and another begins.
    let outline = build_runs(&[at(0, 0), at(100_000, 0), at(0, 0)], width).unwrap();
    assert_eq!(paths(&outline).len(), 5);
    // A gentle curve under a brush far wider than its steps: the inner side folds at every point, so every
    // piece is a run of its own, and the joints are wedges of three straight edges, not discs.
    let curl: Vec<InkPoint> = (0..40).map(|i| at(i * 1_000, i * i * 12)).collect();
    let outline = build_runs(&curl, 40 << 16).unwrap();
    let curves = outline
        .segs
        .iter()
        .filter(|s| matches!(s, Seg::Quad(..)))
        .count();
    assert_eq!(curves, 32, "only the two ends are discs");
    assert!(outline.segs.len() <= 32 + 7 * 39, "{}", outline.segs.len());
    // The same errors as `build`.
    assert_eq!(build_runs(&[], width).err(), build(&[], width).err());
    assert_eq!(build_runs(&line, 0).err(), build(&line, 0).err());
}
