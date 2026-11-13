//! The scene the renderer trials draw: pen strokes spread over a 3840 x 2160 view, as paths in view pixels, and
//! the tiles of 512 pixels they touch. `spike/render-trial` (native) and `spike/render-web` (the browser) both
//! build it from this file, so they draw the same thing.

use std::f64::consts::FRAC_1_SQRT_2;

use everink_core::outline::{
    InkPoint, Outline, Seg, build, build_runs, build_runs_within, build_single_path,
};
use vello_common::kurbo::{BezPath, Point, Rect, Shape};
use vello_common::peniko::Color;

pub const VIEW: (u32, u32) = (3840, 2160);
pub const TILE: u16 = 512;

/// The forms of outline, with the names the runs print. The first is what the others are compared with, and
/// the browser trial keeps the tiles of the last as its cache.
pub const KINDS: [(&str, Kind); 6] = [
    ("discs and bands (slice 3a's builder)", Kind::DiscsAndBands),
    ("octagons and bands (this trial only)", Kind::Octagons),
    (
        "single path (the design's, which leaves holes)",
        Kind::SinglePath,
    ),
    ("runs, cut at a quarter turn (slice 3d)", Kind::Runs),
    (
        "runs, cut at an eighth of a turn (slice 3d)",
        Kind::RunsEighth,
    ),
    (
        "runs, cut at an eighth of a turn, sides smoothed (slice 3d)",
        Kind::RunsSmooth,
    ),
];

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One object of the scene: its path in view pixels, its box, and its colour.
pub struct Object {
    pub path: BezPath,
    pub bounds: Rect,
    pub colour: Color,
    pub segments: usize,
}

/// An outline of the 2^-40 lattice as a path in view pixels: `scale` pixels per object unit, moved to `at`.
fn to_path(outline: &Outline, scale: f64, at: (f64, f64)) -> BezPath {
    let px = |p: everink_core::outline::LPt| {
        Point::new(
            p.x as f64 / 1_099_511_627_776.0 * scale + at.0,
            p.y as f64 / 1_099_511_627_776.0 * scale + at.1,
        )
    };
    let mut path = BezPath::new();
    let mut end = None;
    for seg in &outline.segs {
        if end != Some(seg.start()) {
            if end.is_some() {
                path.close_path();
            }
            path.move_to(px(seg.start()));
        }
        match seg {
            Seg::Line(_, b) => path.line_to(px(*b)),
            Seg::Quad(_, c, b) => path.quad_to(px(*c), px(*b)),
        }
        end = Some(seg.end());
    }
    path.close_path();
    path
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// `everink_core::outline::build`: a sixteen-cornered disc of curves at every point and a band between.
    DiscsAndBands,
    /// `everink_core::outline::build_single_path`: the design's one path, which leaves holes.
    SinglePath,
    /// `everink_core::outline::build_runs`: one straight-edged path for each stretch that turns gently, and a
    /// disc where one ends. No holes.
    Runs,
    /// The same with a run ended at an eighth of a turn and not a quarter.
    RunsEighth,
    /// That with the sides of each run smoothed, as the single path's are.
    RunsSmooth,
    /// For this trial only: the same union with straight edges, an octagon at every point and a band between.
    /// It is built in plain floats here, so it is a figure for the renderer's cost and not a builder to keep.
    Octagons,
}

/// The union of an octagon at every point and a band along every piece, all anticlockwise, with straight edges.
fn octagons(points: &[InkPoint], width: i32, scale: f64, at: (f64, f64)) -> BezPath {
    const TURN: [(f64, f64); 8] = [
        (1.0, 0.0),
        (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        (0.0, 1.0),
        (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        (-1.0, 0.0),
        (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
        (0.0, -1.0),
        (FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    ];
    let px: Vec<(Point, f64)> = points
        .iter()
        .map(|p| {
            // The same pressure curve as the core, near enough for a timing: a quarter to the whole width.
            let share = 0.25 + 0.75 * f64::from(p.pressure) / 255.0;
            (
                Point::new(
                    f64::from(p.x) / 65536.0 * scale + at.0,
                    f64::from(p.y) / 65536.0 * scale + at.1,
                ),
                f64::from(width) / 65536.0 * scale / 2.0 * share,
            )
        })
        .collect();
    let mut path = BezPath::new();
    for (i, &(p, r)) in px.iter().enumerate() {
        path.move_to(Point::new(p.x + r, p.y));
        for (c, s) in &TURN[1..] {
            path.line_to(Point::new(p.x + r * c, p.y + r * s));
        }
        path.close_path();
        let Some(&(q, s)) = px.get(i + 1) else {
            break;
        };
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 0.0 {
            let (nx, ny) = (-dy / len, dx / len);
            path.move_to(Point::new(p.x - nx * r, p.y - ny * r));
            path.line_to(Point::new(q.x - nx * s, q.y - ny * s));
            path.line_to(Point::new(q.x + nx * s, q.y + ny * s));
            path.line_to(Point::new(p.x + nx * r, p.y + ny * r));
            path.close_path();
        }
    }
    path
}

/// One stroke of the scene before it is outlined.
struct Stroke {
    points: Vec<InkPoint>,
    width: i32,
    at: (f64, f64),
    colour: u64,
}

/// A pen stroke: a random walk of 30 to 120 points, about 60 to 250 pixels long at 256 pixels a unit, 2 to 6
/// pixels wide, somewhere in the view. A rough one also turns sharply, doubles back or pauses at about one
/// point in fifteen, as the corpus of the core's tests does. (A stroke that is not rough draws the same numbers
/// as it did before roughness was added, so the scene of slices 3b and 3c is unchanged.)
fn stroke(rng: &mut Rng, rough: bool) -> Stroke {
    let n = 30 + rng.below(91) as usize;
    let (mut x, mut y) = (0i32, 0i32);
    let (mut dx, mut dy) = (rng.below(801) as i32 - 400, rng.below(801) as i32 - 400);
    let points: Vec<InkPoint> = (0..n)
        .map(|_| {
            if rough && rng.below(15) == 0 {
                (dx, dy) = match rng.below(4) {
                    0 => (-dx, -dy),                   // straight back
                    1 => (-dy, dx),                    // a right angle
                    2 => (-dx + dy / 8, -dy - dx / 8), // a hairpin
                    _ => (0, 0),                       // a pause
                };
            }
            dx = (dx + rng.below(201) as i32 - 100).clamp(-600, 600);
            dy = (dy + rng.below(201) as i32 - 100).clamp(-600, 600);
            x += dx;
            y += dy;
            InkPoint {
                x,
                y,
                pressure: 120 + rng.below(100) as u8,
            }
        })
        .collect();
    // A point lattice unit is 2^-16 of an object unit; 400 of them a sample is 0.006 units. At 256
    // pixels a unit a sample moves about 1.5 pixels and the stroke is 2 to 6 pixels wide.
    let width = ((2 + rng.below(5)) as i32) << 8; // 2 to 6 pixels at 256 pixels a unit
    let at = (
        rng.below(u64::from(VIEW.0)) as f64,
        rng.below(u64::from(VIEW.1)) as f64,
    );
    Stroke {
        points,
        width,
        at,
        colour: rng.next(),
    }
}

/// The outline of a stroke in one of the forms, as a path at `scale` pixels a unit, moved to `at`.
fn outlined(kind: Kind, points: &[InkPoint], width: i32, scale: f64, at: (f64, f64)) -> BezPath {
    let outline = match kind {
        Kind::DiscsAndBands => build(points, width),
        Kind::SinglePath => build_single_path(points, width),
        Kind::Runs => build_runs(points, width),
        Kind::RunsEighth => build_runs_within(points, width, FRAC_1_SQRT_2, false),
        Kind::RunsSmooth => build_runs_within(points, width, FRAC_1_SQRT_2, true),
        Kind::Octagons => return octagons(points, width, scale, at),
    };
    to_path(&outline.expect("a stroke"), scale, at)
}

/// A scene of pen strokes spread over the view, at 256 pixels a unit.
pub fn scene(objects: usize, kind: Kind) -> Vec<Object> {
    let mut rng = Rng(7);
    (0..objects)
        .map(|_| {
            let s = stroke(&mut rng, false);
            let path = outlined(kind, &s.points, s.width, 256.0, s.at);
            let bounds = path.bounding_box();
            let c = s.colour;
            Object {
                segments: path.segments().count(),
                path,
                bounds,
                colour: Color::from_rgba8(
                    c as u8 / 2,
                    (c >> 8) as u8 / 2,
                    (c >> 16) as u8 / 2,
                    255,
                ),
            }
        })
        .collect()
}

/// The same strokes one by one, each at the origin, for comparing what two forms of outline cover. `share` is
/// the brush's width in hundredths of the stroke's own: under 100 the stroke is built thinner. A steady stroke
/// has one pressure all along; the scene's changes at random from point to point, which no pen does.
#[allow(dead_code)] // the browser trial builds this file in and does not compare the forms
pub fn lone_strokes(
    count: usize,
    kind: Kind,
    scale: f64,
    (rough, steady): (bool, bool),
    share: i32,
) -> Vec<BezPath> {
    let mut rng = Rng(7);
    (0..count)
        .map(|_| {
            let mut s = stroke(&mut rng, rough);
            if steady {
                s.points.iter_mut().for_each(|p| p.pressure = 170);
            }
            outlined(kind, &s.points, s.width * share / 100, scale, (0.0, 0.0))
        })
        .collect()
}

/// The objects that touch each tile, in drawing order.
pub fn buckets(objects: &[Object]) -> Vec<((u32, u32), Vec<usize>)> {
    let (cols, rows) = (
        VIEW.0.div_ceil(u32::from(TILE)),
        VIEW.1.div_ceil(u32::from(TILE)),
    );
    let mut tiles = Vec::new();
    for ty in 0..rows {
        for tx in 0..cols {
            let tile = Rect::new(
                f64::from(tx * u32::from(TILE)),
                f64::from(ty * u32::from(TILE)),
                f64::from((tx + 1) * u32::from(TILE)),
                f64::from((ty + 1) * u32::from(TILE)),
            );
            let touching = objects
                .iter()
                .enumerate()
                .filter(|(_, o)| o.bounds.overlaps(tile))
                .map(|(i, _)| i)
                .collect();
            tiles.push(((tx, ty), touching));
        }
    }
    tiles
}
