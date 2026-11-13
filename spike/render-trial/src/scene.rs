//! The scene the renderer trials draw: pen strokes spread over a 3840 x 2160 view, as paths in view pixels, and
//! the tiles of 512 pixels they touch. `spike/render-trial` (native) and `spike/render-web` (the browser) both
//! build it from this file, so they draw the same thing.

use std::f64::consts::FRAC_1_SQRT_2;

use everink_core::outline::{InkPoint, Outline, Seg, build};
use vello_common::kurbo::{BezPath, Point, Rect, Shape};
use vello_common::peniko::Color;

pub const VIEW: (u32, u32) = (3840, 2160);
pub const TILE: u16 = 512;

/// The forms of outline, with the names the runs print. The browser trial keeps the tiles of the last as its
/// cache, so the outline Everink uses is the last.
///
/// Slices 3b to 3d drew more forms here (a disc at every point with bands between, the design's single path,
/// and runs with other settings) to choose among them. The owner chose on 2026-11-13, the core now has the one
/// builder, and those forms and the runs that compared them went with the others; they are in the repository's
/// history, last at commit `4b84a65`.
pub const KINDS: [(&str, Kind); 2] = [
    ("octagons and bands (this trial only)", Kind::Octagons),
    ("runs, smoothed sides (the core's outline)", Kind::Runs),
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
    /// `everink_core::outline::build`: runs with smoothed sides, the outline Everink uses.
    Runs,
    /// For this trial only: an octagon at every point and a band between, with straight edges. It is built in
    /// plain floats here and has several times the segments, so it shows how the renderer's cost moves with
    /// the outline's form; it is not a builder to keep.
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

/// A scene of pen strokes: each a random walk of 30 to 120 points, about 60 to 250 pixels long, 2 to 6 pixels
/// wide, somewhere in the view.
pub fn scene(objects: usize, kind: Kind) -> Vec<Object> {
    let mut rng = Rng(7);
    (0..objects)
        .map(|_| {
            let n = 30 + rng.below(91) as usize;
            let (mut x, mut y) = (0i32, 0i32);
            let (mut dx, mut dy) = (rng.below(801) as i32 - 400, rng.below(801) as i32 - 400);
            let points: Vec<InkPoint> = (0..n)
                .map(|_| {
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
            let path = match kind {
                Kind::Runs => to_path(&build(&points, width).expect("a stroke"), 256.0, at),
                Kind::Octagons => octagons(&points, width, 256.0, at),
            };
            let bounds = path.bounding_box();
            let c = rng.next();
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
