//! The outline of a stroke: from the stored points to a closed, filled path (design section 06, "Outline
//! algorithm, version 1").
//!
//! Every client builds the outline from the same stored points and must get the same path, bit for bit, on every
//! machine: the outline is what the cut consumes, what bounds come from, and what two people point at. So this
//! file keeps to arithmetic that IEEE 754 defines exactly and that every target gives the same answer for:
//!
//! - `+ - * /` and `sqrt` on `f64`, comparisons, `round_ties_even`, and casts;
//! - no `mul_add`, no `sin`/`cos`/`atan2`/`powf`/`hypot`, no float `min`/`max`/`clamp` (`clippy.toml` forbids them
//!   in this crate);
//! - tables are literals, loops have a fixed order, and nothing is summed in parallel.
//!
//! The result is put on a lattice of 2^-40 of an object unit as the last step, and it is the lattice outline,
//! not the floats on the way, that is promised to be identical.
//!
//! What this version leaves out is listed in `docs/spike/results-3a-wasm-outlines-cut.md`.

use std::f64::consts::FRAC_1_SQRT_2;

/// A stored point of a stroke: 2^-16 of an object unit, and a pressure byte (0 = the device gave none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InkPoint {
    pub x: i32,
    pub y: i32,
    pub pressure: u8,
}

/// A point of an outline: 2^-40 of an object unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LPt {
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Seg {
    Line(LPt, LPt),
    /// A quadratic Bézier: start, control, end.
    Quad(LPt, LPt, LPt),
}

impl Seg {
    pub fn start(&self) -> LPt {
        match self {
            Seg::Line(a, _) | Seg::Quad(a, _, _) => *a,
        }
    }

    pub fn end(&self) -> LPt {
        match self {
            Seg::Line(_, b) | Seg::Quad(_, _, b) => *b,
        }
    }
}

/// One or more closed paths, one after another, filled with the non-zero rule. A new closed path begins where a
/// segment does not start at the end of the one before it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Outline {
    pub segs: Vec<Seg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineError {
    /// No points.
    Empty,
    /// The width is not positive.
    BadWidth,
}

/// The version of this algorithm. A stroke stores the version it was drawn with (`style_ver`), so a later
/// version cannot change how an old stroke looks.
pub const STYLE_VERSION: u8 = 1;

/// Lattice steps per object unit, for points (2^16) and for outlines (2^40).
const POINT_STEPS: f64 = 65536.0;
const OUTLINE_STEPS: f64 = 1_099_511_627_776.0;

/// Pressure to width: the share of the brush's width at 17 pressures from none to full, as literals. Between two
/// entries the share is interpolated with `+ - * /`. Every entry is a multiple of 1/64, so the table itself is
/// exact. A light touch gives a quarter of the width and full pressure all of it, rising faster at first.
const WIDTH_SHARE: [f64; 17] = [
    0.25, 0.328125, 0.40625, 0.46875, 0.53125, 0.59375, 0.640625, 0.6875, 0.734375, 0.78125,
    0.8125, 0.84375, 0.875, 0.90625, 0.9375, 0.96875, 1.0,
];

/// Cosine and sine of 22.5, 45, 67.5, 90, 112.5, 135 and 157.5 degrees, as literals. A round join or cap is
/// made by turning a normal through these with multiplications; no `sin` or `cos` is ever called.
const TURN: [(f64, f64); 7] = [
    (0.923_879_532_511_286_7, 0.382_683_432_365_089_8),
    (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (0.382_683_432_365_089_8, 0.923_879_532_511_286_7),
    (0.0, 1.0),
    (-0.382_683_432_365_089_8, 0.923_879_532_511_286_7),
    (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (-0.923_879_532_511_286_7, 0.382_683_432_365_089_8),
];

#[derive(Clone, Copy)]
struct V {
    x: f64,
    y: f64,
}

impl V {
    fn add(self, o: V) -> V {
        V {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }

    fn sub(self, o: V) -> V {
        V {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }

    fn scale(self, k: f64) -> V {
        V {
            x: self.x * k,
            y: self.y * k,
        }
    }

    fn dot(self, o: V) -> f64 {
        self.x * o.x + self.y * o.y
    }

    fn cross(self, o: V) -> f64 {
        self.x * o.y - self.y * o.x
    }

    /// Turned a quarter of a turn to the left.
    fn left(self) -> V {
        V {
            x: -self.y,
            y: self.x,
        }
    }

    /// Turned by an angle whose cosine and sine are given. `sign` is 1 for a left turn and -1 for a right one.
    fn turned(self, (c, s): (f64, f64), sign: f64) -> V {
        let s = s * sign;
        V {
            x: self.x * c - self.y * s,
            y: self.x * s + self.y * c,
        }
    }

    /// The same direction with length 1, or `None` for a zero vector.
    fn unit(self) -> Option<V> {
        let len = (self.x * self.x + self.y * self.y).sqrt();
        if len > 0.0 {
            Some(V {
                x: self.x / len,
                y: self.y / len,
            })
        } else {
            None
        }
    }
}

fn share(pressure: u8) -> f64 {
    if pressure == 0 {
        return 1.0; // no pressure from the device: the full width (width from speed is not built)
    }
    // 1..=255 onto the 16 intervals of the table.
    let at = f64::from(pressure - 1) * 16.0 / 254.0;
    let i = at as usize; // 0..=16
    if i >= 16 {
        return WIDTH_SHARE[16];
    }
    let f = at - i as f64;
    WIDTH_SHARE[i] + (WIDTH_SHARE[i + 1] - WIDTH_SHARE[i]) * f
}

fn lattice(v: V) -> LPt {
    // Points are within 2^15 object units and a width is far below that, so the product is below 2^56: inside
    // an i64 and exact as an f64 product, since multiplying by a power of two changes only the exponent.
    LPt {
        x: (v.x * OUTLINE_STEPS).round_ties_even() as i64,
        y: (v.y * OUTLINE_STEPS).round_ties_even() as i64,
    }
}

/// Step 1, shared by both builders: the points in object units with their radius. A point equal to the one
/// before it carries no direction and is left out; its pressure is kept if it is the greater, so a pause that
/// presses harder still widens the stroke.
fn centre_line(points: &[InkPoint], base_width: i32) -> Result<Vec<(V, f64)>, OutlineError> {
    if base_width <= 0 {
        return Err(OutlineError::BadWidth);
    }
    let half = f64::from(base_width) / POINT_STEPS / 2.0;
    let mut centre: Vec<(V, f64)> = Vec::with_capacity(points.len());
    let mut last: Option<InkPoint> = None;
    for p in points {
        let r = half * share(p.pressure);
        if let Some(l) = last
            && l.x == p.x
            && l.y == p.y
        {
            let prev = centre.last_mut().expect("a last point has an entry");
            if r > prev.1 {
                prev.1 = r;
            }
            continue;
        }
        last = Some(*p);
        centre.push((
            V {
                x: f64::from(p.x) / POINT_STEPS,
                y: f64::from(p.y) / POINT_STEPS,
            },
            r,
        ));
    }
    if centre.is_empty() {
        return Err(OutlineError::Empty);
    }
    Ok(centre)
}

/// The sixteen corners of a disc, anticlockwise.
fn disc(p: V, r: f64, corners: &mut Vec<V>) {
    let n = V { x: 0.0, y: 1.0 };
    corners.push(p.add(n.scale(r)));
    for t in TURN {
        corners.push(p.add(n.turned(t, 1.0).scale(r)));
    }
    corners.push(p.sub(n.scale(r)));
    for t in TURN {
        corners.push(p.sub(n.turned(t, 1.0).scale(r)));
    }
}

/// Smooths a closed polygon and puts it on the lattice: each corner becomes the control point of a quadratic
/// curve between the midpoints of its two edges (a midpoint is exact in binary arithmetic). A curve whose three
/// points fall on one lattice point is dropped. The last curve ends exactly where the first began.
fn smooth_closed(corners: &[V], segs: &mut Vec<Seg>) {
    let count = corners.len();
    let mid = |a: V, b: V| V {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    };
    let first = lattice(mid(corners[count - 1], corners[0]));
    let mut from = first;
    for j in 0..count {
        let corner = corners[j];
        let to = if j + 1 == count {
            first
        } else {
            lattice(mid(corner, corners[(j + 1) % count]))
        };
        let control = lattice(corner);
        if !(from == control && control == to) {
            segs.push(Seg::Quad(from, control, to));
        }
        from = to;
    }
}

/// Builds the outline of a stroke. `base_width` is the brush's full width in 16.16 fixed point object units.
///
/// The outline is the union of simple pieces, each a closed path of its own, all going the same way round: a
/// disc at every point, and a band along every piece of the centre line from one disc to the next. Filled with
/// the non-zero rule, pieces that overlap add up and never cancel, so the stroke has no holes however tightly
/// it curls or doubles back. (The single path of the design does have them; see `build_single_path`.)
pub fn build(points: &[InkPoint], base_width: i32) -> Result<Outline, OutlineError> {
    let centre = centre_line(points, base_width)?;
    let mut segs = Vec::with_capacity(centre.len() * 20);
    let mut corners: Vec<V> = Vec::with_capacity(16);
    for (i, &(p, r)) in centre.iter().enumerate() {
        corners.clear();
        disc(p, r, &mut corners);
        smooth_closed(&corners, &mut segs);
        let Some(&(q, s)) = centre.get(i + 1) else {
            break;
        };
        // The band: the right-hand edge forward, the left-hand edge back. That is anticlockwise, like the discs,
        // whichever way the piece points. Its four corners are on the lattice, and its edges are straight.
        let n = q.sub(p).unit().expect("two different points").left();
        let band = [
            lattice(p.sub(n.scale(r))),
            lattice(q.sub(n.scale(s))),
            lattice(q.add(n.scale(s))),
            lattice(p.add(n.scale(r))),
        ];
        for k in 0..4 {
            let (a, b) = (band[k], band[(k + 1) % 4]);
            if a != b {
                segs.push(Seg::Line(a, b));
            }
        }
    }
    Ok(Outline { segs })
}

/// The outline as section 06 first specified it: one closed path, up the left side of the stroke, round the far
/// end, down the right side and round the near end, with a round join on the outside of a sharp turn.
///
/// Kept to record what the spike found: where a stroke doubles back, or curls tightly under a wide brush, the
/// two sides cross and the non-zero rule leaves a hole over the stroke's own centre line
/// (`tests/outlines.rs`). Everink uses `build`.
pub fn build_single_path(points: &[InkPoint], base_width: i32) -> Result<Outline, OutlineError> {
    let centre = centre_line(points, base_width)?;

    // The polygon, as corners in order.
    let mut corners: Vec<V> = Vec::with_capacity(centre.len() * 2 + 16);
    if centre.len() == 1 {
        let (p, r) = centre[0];
        disc(p, r, &mut corners);
    } else {
        // Step 3: the direction of each piece of the centre line. No piece is empty, because equal neighbours
        // were left out.
        let dirs: Vec<V> = centre
            .windows(2)
            .map(|w| w[1].0.sub(w[0].0).unit().expect("two different points"))
            .collect();
        let n = centre.len();
        // Steps 4 and 5: one side, with a round join where the path turns away from that side by more than a
        // quarter turn, then the round cap at its end. `side` is 1 for the left side going forward and -1 for
        // the right side coming back.
        let side = |side: f64, corners: &mut Vec<V>| {
            for step in 0..n {
                let i = if side > 0.0 { step } else { n - 1 - step };
                let (p, r) = centre[i];
                // The directions into and out of this point, in the order this side is walked.
                let (into, out) = if side > 0.0 {
                    (i.checked_sub(1).map(|j| dirs[j]), dirs.get(i).copied())
                } else {
                    (
                        dirs.get(i).map(|d| d.scale(-1.0)),
                        i.checked_sub(1).map(|j| dirs[j].scale(-1.0)),
                    )
                };
                match (into, out) {
                    (Some(a), Some(b)) => {
                        let (na, nb) = (a.left(), b.left());
                        let turn = a.cross(b);
                        if a.dot(b) < 0.0 && turn < 0.0 {
                            // A sharp turn to the right: this (left-hand) side is the outside of the bend.
                            // Round it, from the normal of the piece before to the normal of the piece after.
                            corners.push(p.add(na.scale(r)));
                            for t in TURN {
                                let v = na.turned(t, -1.0);
                                if na.cross(v) < 0.0 && v.cross(nb) < 0.0 {
                                    corners.push(p.add(v.scale(r)));
                                }
                            }
                            corners.push(p.add(nb.scale(r)));
                        } else {
                            // Otherwise one corner, on the normal of the two pieces taken together. The sum
                            // is zero only when the path doubles straight back, and then either normal does.
                            let m = na.add(nb).unit().unwrap_or(na);
                            corners.push(p.add(m.scale(r)));
                        }
                    }
                    (None, Some(b)) => corners.push(p.add(b.left().scale(r))),
                    (Some(a), None) => {
                        // The end of this side: the corner, then the cap, half a turn to the right.
                        let na = a.left();
                        corners.push(p.add(na.scale(r)));
                        for t in TURN {
                            corners.push(p.add(na.turned(t, -1.0).scale(r)));
                        }
                    }
                    (None, None) => unreachable!("two or more points"),
                }
            }
        };
        side(1.0, &mut corners);
        side(-1.0, &mut corners);
    }

    // Step 6: smooth. Step 7: onto the lattice.
    let mut segs = Vec::with_capacity(corners.len());
    smooth_closed(&corners, &mut segs);
    Ok(Outline { segs })
}
