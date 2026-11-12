//! Cutting a large shape down to the view (design section 01, "The cut").
//!
//! A shape drawn far above the camera's level is astronomically larger than the window: 1024 times per level.
//! Its coordinates do not fit a float, and only the sliver that crosses the window matters. The cut finds that
//! sliver with whole-number arithmetic: it halves a curve again and again (de Casteljau at one half, which
//! needs only additions and a shift), throws away each half whose box misses the window, and stops when a piece
//! is small enough to be given to the renderer as floats.
//!
//! Coordinates here are **relative to the centre of the window**, in steps of 2^-40 of a camera-level unit. The
//! caller gets them by moving the shape's lattice points to the camera's level (a shift of ten bits per level)
//! and subtracting the window's centre; `place` does it. Each halving rounds toward minus infinity, so a piece
//! is off the true curve by at most one step per halving, far below a pixel.
//!
//! Two shortcuts keep the work near `10 x levels` halvings of two or three numbers:
//!
//! - A piece whose control point is within a tolerance of its chord is a straight line from then on, and a line
//!   is halved with one addition per axis.
//! - A piece whose box misses the window is replaced by its chord, with both ends pulled onto the edge of a box
//!   sixteen times the window, so that the path stays closed and winds round every point of the window as the
//!   true shape does.
//!
//! What this version leaves out is listed in `docs/spike/results-3a-wasm-outlines-cut.md`.

use std::cmp::Ordering;

use crate::Axis;

/// Steps per camera-level unit in a cut piece: 2^40.
const STEPS: f64 = 1_099_511_627_776.0;
/// A far piece is pulled onto a box this many times the window.
const FAR_BOX: i128 = 16;
/// A curve is a line once its control point is within the window's side shifted right by this many bits of its
/// chord: 2^-24 of the window.
const FLAT_SHIFT: u32 = 24;

/// A point relative to the window's centre, in steps of 2^-40 of a camera-level unit. May be very large.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigPt {
    pub x: Axis,
    pub y: Axis,
}

/// A piece of a cut shape, relative to the window's centre, in camera-level units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    Line([f64; 2], [f64; 2]),
    Quad([f64; 2], [f64; 2], [f64; 2]),
    /// The chord of a part that misses the window, both ends on or inside the far box. It is there to keep the
    /// path closed; nothing of it is visible.
    Far([f64; 2], [f64; 2]),
}

impl Piece {
    pub fn start(&self) -> [f64; 2] {
        match self {
            Piece::Line(a, _) | Piece::Quad(a, _, _) | Piece::Far(a, _) => *a,
        }
    }

    pub fn end(&self) -> [f64; 2] {
        match self {
            Piece::Line(_, b) | Piece::Quad(_, _, b) | Piece::Far(_, b) => *b,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CutStats {
    /// Times a piece was halved.
    pub halvings: u32,
    /// The deepest a piece was halved.
    pub depth: u32,
}

/// Moves a point of a shape's lattice (2^-40 of a unit of the shape's level) to the camera's level, `levels`
/// finer, and makes it relative to the window's centre (given on the camera's level, in the same steps).
pub fn place(x: i64, y: i64, levels: u32, centre: &BigPt) -> BigPt {
    let up = |v: i64, c: &Axis| Axis::from_i128(i128::from(v)).shl(10 * levels).sub(c);
    BigPt {
        x: up(x, &centre.x),
        y: up(y, &centre.y),
    }
}

fn half(a: &Axis, b: &Axis) -> Axis {
    a.add(b).shr_floor(1)
}

fn mid(a: &BigPt, b: &BigPt) -> BigPt {
    BigPt {
        x: half(&a.x, &b.x),
        y: half(&a.y, &b.y),
    }
}

/// A coordinate as a float, in camera-level units. The caller has made sure it is small.
fn unit(a: &Axis) -> f64 {
    a.to_i128().expect("a piece near the window is small") as f64 / STEPS
}

fn near(p: &BigPt) -> [f64; 2] {
    [unit(&p.x), unit(&p.y)]
}

/// The point pulled onto the far box, as floats.
fn far(p: &BigPt, limit: i128) -> [f64; 2] {
    let pull = |a: &Axis| {
        let v = match a.to_i128() {
            Some(v) if v > limit => limit,
            Some(v) if v < -limit => -limit,
            Some(v) => v,
            None if a.signum() > 0 => limit,
            None => -limit,
        };
        v as f64 / STEPS
    };
    [pull(&p.x), pull(&p.y)]
}

/// The least and the greatest of a piece's coordinates on one axis.
struct Bounds<'a> {
    lo: &'a Axis,
    hi: &'a Axis,
}

impl Bounds<'_> {
    /// True when every coordinate is on one side of the window.
    fn misses(&self, window: &Bounds<'_>) -> bool {
        self.lo.cmp(window.hi) == Ordering::Greater || self.hi.cmp(window.lo) == Ordering::Less
    }
}

fn bounds<'a>(values: impl Iterator<Item = &'a Axis>) -> Bounds<'a> {
    let mut it = values;
    let first = it.next().expect("a piece has points");
    let (mut lo, mut hi) = (first, first);
    for v in it {
        if v.cmp(lo) == Ordering::Less {
            lo = v;
        }
        if v.cmp(hi) == Ordering::Greater {
            hi = v;
        }
    }
    Bounds { lo, hi }
}

/// Cuts one quadratic curve to the window: the square of half-side `half_window` (in steps of 2^-40 of a
/// camera-level unit) round the origin. The pieces are appended to `out` in order along the curve, each starting
/// where the one before ended.
///
/// The window is whatever the caller wants prepared: section 01's expanded window, not the bare viewport, so
/// that a small pan needs no new cut.
pub fn cut_quad(curve: [BigPt; 3], half_window: i64, out: &mut Vec<Piece>) -> CutStats {
    let h = i128::from(half_window.max(1));
    let (window_lo, window_hi) = (Axis::from_i128(-h), Axis::from_i128(h));
    let window = Bounds {
        lo: &window_lo,
        hi: &window_hi,
    };
    // A piece is small enough when its box is no more than four windows across.
    let small = Axis::from_i128(8 * h);
    let flat = Axis::from_i128(((2 * h) >> FLAT_SHIFT).max(1) * 4);
    let far_limit = FAR_BOX * h;
    let mut stats = CutStats::default();

    // The pieces still to look at, the next one along the curve on top. A `None` control point is a line.
    let [p0, p1, p2] = curve;
    let mut todo: Vec<(BigPt, Option<BigPt>, BigPt, u32)> = vec![(p0, Some(p1), p2, 0)];
    while let Some((a, control, b, depth)) = todo.pop() {
        stats.depth = stats.depth.max(depth);
        let xs = bounds(
            [&a.x, &b.x]
                .into_iter()
                .chain(control.as_ref().map(|c| &c.x)),
        );
        let ys = bounds(
            [&a.y, &b.y]
                .into_iter()
                .chain(control.as_ref().map(|c| &c.y)),
        );
        if xs.misses(&window) || ys.misses(&window) {
            let piece = Piece::Far(far(&a, far_limit), far(&b, far_limit));
            if piece.start() != piece.end() {
                out.push(piece);
            }
            continue;
        }
        let across = |b: &Bounds<'_>| b.hi.sub(b.lo).cmp(&small) != Ordering::Greater;
        if across(&xs) && across(&ys) {
            out.push(match &control {
                Some(c) => Piece::Quad(near(&a), near(c), near(&b)),
                None => Piece::Line(near(&a), near(&b)),
            });
            continue;
        }
        stats.halvings += 1;
        match control {
            Some(c) => {
                // How far the curve is from its chord: a quarter of `a - 2c + b`, which a halving divides by 4.
                let bend = |a: &Axis, c: &Axis, b: &Axis| a.add(b).sub(&c.shl(1));
                let within = |v: Axis| {
                    v.cmp(&flat) != Ordering::Greater
                        && Axis::ZERO.sub(&v).cmp(&flat) != Ordering::Greater
                };
                if within(bend(&a.x, &c.x, &b.x)) && within(bend(&a.y, &c.y, &b.y)) {
                    // Straight from here on.
                    let m = mid(&a, &b);
                    todo.push((m.clone(), None, b, depth + 1));
                    todo.push((a, None, m, depth + 1));
                } else {
                    let (ac, cb) = (mid(&a, &c), mid(&c, &b));
                    let m = mid(&ac, &cb);
                    todo.push((m.clone(), Some(cb), b, depth + 1));
                    todo.push((a, Some(ac), m, depth + 1));
                }
            }
            None => {
                let m = mid(&a, &b);
                todo.push((m.clone(), None, b, depth + 1));
                todo.push((a, None, m, depth + 1));
            }
        }
    }
    stats
}
