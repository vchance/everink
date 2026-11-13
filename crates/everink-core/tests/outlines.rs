//! Stroke outlines: the same on every machine, closed, and covering the stroke they were built from.
//!
//! `tests/golden/outlines.txt` holds one line per stroke of a fixed corpus: the number of segments and a hash of
//! every coordinate of the outline. It was written on one machine (aarch64) and must pass byte for byte on the
//! others: x86-64 as it is built by default and with its fused multiply-add switched on, and wasm32. aarch64
//! always has a fused multiply-add, so the file itself was written where the compiler could have used one.
//! Rewrite it only when the outline algorithm is meant to change:
//! `UPDATE_GOLDEN=1 cargo test -p everink-core --test outlines`.

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use common::Rng;
use everink_core::outline::{InkPoint, LPt, Outline, OutlineError, STYLE_VERSION, Seg, build};

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
        "# everink-core stroke outlines (runs, smoothed sides), style version {STYLE_VERSION}"
    )
    .unwrap();
    writeln!(built, "# o <stroke> <points> <segments> <hash>").unwrap();
    for (i, (points, width)) in corpus().iter().enumerate() {
        let outline = build(points, *width).unwrap();
        writeln!(
            built,
            "o {i} {} {} {:016x}",
            points.len(),
            outline.segs.len(),
            hash(&outline)
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
fn a_dot_is_a_circle_and_bad_input_is_refused() {
    let dot = [InkPoint {
        x: 1000,
        y: -2000,
        pressure: 255,
    }];
    {
        let outline = build(&dot, 4 << 16).unwrap();
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
    assert_eq!(build(&[], 1 << 16), Err(OutlineError::Empty));
    assert_eq!(build(&dot, 0), Err(OutlineError::BadWidth));
    assert_eq!(build(&dot, -5), Err(OutlineError::BadWidth));
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
/// points of the curve itself, in whole numbers. Where a curve bulges outward its chords are inside it, and where
/// it bends inward they are outside it by less than a sixty-fourth of the curve's own depth.
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

/// `winding` over the polygons of `fine_paths`. A polygon does not wind round a point outside its box. No
/// polygon may wind the wrong way round the point: one that did would take away what another covers.
fn fine_winding(polygons: &[Boxed], p: LPt) -> i32 {
    polygons
        .iter()
        .filter(|(_, b)| b[0] <= p.x && p.x <= b[2] && b[1] <= p.y && p.y <= b[3])
        .map(|(polygon, _)| {
            let w = winding(std::slice::from_ref(polygon), p);
            assert!(w >= 0, "a path winds clockwise round a point");
            w
        })
        .sum()
}

#[test]
fn an_outline_is_closed_covers_its_stroke_and_is_light() {
    let corpus = corpus();
    let (mut checked, mut segments, mut paths_made, mut stored) = (0, 0, 0, 0);
    for (i, (points, width)) in corpus.iter().enumerate() {
        let outline = build(points, *width).unwrap();
        closed_and_in_reach(i, points, *width, &outline);
        segments += outline.segs.len();
        stored += points.len();
        let polygons = fine_paths(&outline);
        paths_made += polygons.len();
        // Every path goes anticlockwise on the whole. (That no part of one goes the other way round a
        // point is checked at each point below, in `fine_winding`.)
        for (polygon, _) in &polygons {
            assert!(twice_area(polygon) > 0, "stroke {i}: a path goes clockwise");
        }
        // The points of the stroke, and the middle of the piece after each, are inside the outline.
        // Every eighth point is checked, and the first and last, with the curves followed and not read
        // as their control points.
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
                "stroke {i}: point {k} is outside its outline"
            );
            if let Some(q) = points.get(k + 1) {
                let mid = LPt {
                    x: (to_outline(p.x) + to_outline(q.x)) / 2,
                    y: (to_outline(p.y) + to_outline(q.y)) / 2,
                };
                assert!(
                    fine_winding(&polygons, mid) > 0,
                    "stroke {i}: the middle after point {k} is outside its outline"
                );
            }
            checked += 2;
        }
    }
    println!(
        "{segments} segments in {paths_made} paths for {STROKES} strokes of {stored} points in all"
    );
    assert!(checked > 30_000);
    // The corpus is rough, and many of its brushes are far wider than the step between points, which is the
    // costly case: about seven segments a point. A disc at every point and a band between, the form this
    // one replaced, cost twenty a point.
    assert!(
        segments < stored * 10,
        "{segments} segments for {stored} points"
    );
}

#[test]
fn the_small_cases() {
    let at = |x: i32, y: i32| InkPoint {
        x,
        y,
        pressure: 128,
    };
    let width = 1 << 16;
    // One point, and a point given twice: a disc.
    for points in [vec![at(0, 0)], vec![at(5, 5), at(5, 5)]] {
        let outline = build(&points, width).unwrap();
        assert_eq!(paths(&outline).len(), 1);
        assert_eq!(outline.segs.len(), 16);
    }
    // Two points: a disc at each end and one four-cornered run between.
    let outline = build(&[at(0, 0), at(100_000, 0)], width).unwrap();
    assert_eq!(paths(&outline).len(), 3);
    assert_eq!(outline.segs.len(), 16 + 4 + 16);
    // A straight line of many points is still two discs and one run.
    let line: Vec<InkPoint> = (0..50).map(|i| at(i * 10_000, i * 5_000)).collect();
    let outline = build(&line, width).unwrap();
    assert_eq!(paths(&outline).len(), 3);
    // Its corners are in line, so none is rounded: two segments a point and the run's two ends.
    assert_eq!(outline.segs.len(), 16 + 2 * 50 + 16);
    // Straight back: the run ends at the turn, under a disc, and another begins.
    let outline = build(&[at(0, 0), at(100_000, 0), at(0, 0)], width).unwrap();
    assert_eq!(paths(&outline).len(), 5);
    // A gentle curve under a brush far wider than its steps: the inner side folds at every point, so every
    // piece is a run of its own, and the joints are wedges of three straight edges, not discs.
    let curl: Vec<InkPoint> = (0..40).map(|i| at(i * 1_000, i * i * 12)).collect();
    let outline = build(&curl, 40 << 16).unwrap();
    let curves = outline
        .segs
        .iter()
        .filter(|s| matches!(s, Seg::Quad(..)))
        .count();
    assert_eq!(curves, 32, "only the two ends are discs");
    assert!(outline.segs.len() <= 32 + 7 * 39, "{}", outline.segs.len());
    // A gentle curve under a narrow brush is one run with curved sides: a curve at each corner inside it.
    let bend: Vec<InkPoint> = (0..40).map(|i| at(i * 20_000, i * i * 300)).collect();
    let outline = build(&bend, width).unwrap();
    assert_eq!(paths(&outline).len(), 3);
    let curves = outline
        .segs
        .iter()
        .filter(|s| matches!(s, Seg::Quad(..)))
        .count();
    assert_eq!(
        curves,
        32 + 2 * 38,
        "both sides of the run are curved at each of its 38 inner points"
    );
}
