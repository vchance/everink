//! Positions, exact differences, and the one place where an exact difference becomes a float.

use std::cmp::Ordering;

use crate::{Axis, LEVEL_SHIFT, UNIT_BITS};

/// A level. One unit of level `l` is 1024^-l of a level-0 unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(pub i32);

/// A point on the surface. Each axis is a count of steps of 2^-54 of a unit of `level`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub level: Level,
    pub x: Axis,
    pub y: Axis,
}

/// An exact difference of two positions, in steps of 2^-54 of a unit of `level`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub level: Level,
    pub dx: Axis,
    pub dy: Axis,
}

/// A difference as a float. `Far` when its magnitude is 2^1000 units or more; such a thing is never in a view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rel {
    Near(f64),
    /// Carries the sign: -1 or 1.
    Far(i32),
}

impl Rel {
    pub fn near(self) -> Option<f64> {
        match self {
            Rel::Near(v) => Some(v),
            Rel::Far(_) => None,
        }
    }
}

/// The largest level difference this crate will shift across. A position that came from the network passed
/// `check_cap_bytes`, so its level is within the depth cap (1,024 by default) and it never comes near this. The
/// bound exists so that positions built directly cannot ask for a shift of billions of bits.
pub const MAX_SHIFT_LEVELS: i64 = 1 << 20;

fn level_bits(from: Level, to: Level) -> u32 {
    let d = i64::from(to.0) - i64::from(from.0);
    assert!(d >= 0, "level_bits needs to >= from");
    assert!(
        d <= MAX_SHIFT_LEVELS,
        "levels are {d} apart, beyond the {MAX_SHIFT_LEVELS} this crate will shift across"
    );
    (d * i64::from(LEVEL_SHIFT)) as u32
}

impl Position {
    pub fn origin(level: Level) -> Position {
        Position {
            level,
            x: Axis::ZERO,
            y: Axis::ZERO,
        }
    }

    /// The same point on a finer or equal level. Never rounds.
    pub fn at_level(&self, to: Level) -> Position {
        let bits = level_bits(self.level, to);
        Position {
            level: to,
            x: self.x.shl(bits),
            y: self.y.shl(bits),
        }
    }

    /// The point on a coarser or equal level, rounded toward minus infinity. Only the camera and re-levelling use
    /// this; comparisons never do.
    pub fn at_coarser_floor(&self, to: Level) -> Position {
        let bits = level_bits(to, self.level);
        Position {
            level: to,
            x: self.x.shr_floor(bits),
            y: self.y.shr_floor(bits),
        }
    }

    /// Exact difference `self - other`, on the finer of the two levels.
    pub fn sub(&self, other: &Position) -> Delta {
        match self.level.cmp(&other.level) {
            Ordering::Equal => Delta {
                level: self.level,
                dx: self.x.sub(&other.x),
                dy: self.y.sub(&other.y),
            },
            Ordering::Greater => {
                let o = other.at_level(self.level);
                Delta {
                    level: self.level,
                    dx: self.x.sub(&o.x),
                    dy: self.y.sub(&o.y),
                }
            }
            Ordering::Less => {
                let s = self.at_level(other.level);
                Delta {
                    level: other.level,
                    dx: s.x.sub(&other.x),
                    dy: s.y.sub(&other.y),
                }
            }
        }
    }

    /// `self + delta`, on the finer of the two levels. Exact.
    pub fn add_delta(&self, delta: &Delta) -> Position {
        let level = self.level.max(delta.level);
        let p = self.at_level(level);
        let bits = level_bits(delta.level, level);
        Position {
            level,
            x: p.x.add(&delta.dx.shl(bits)),
            y: p.y.add(&delta.dy.shl(bits)),
        }
    }

    /// Exact comparison of the two points on both axes: `(x order, y order)`.
    pub fn cmp_exact(&self, other: &Position) -> (Ordering, Ordering) {
        let level = self.level.max(other.level);
        let a = self.at_level(level);
        let b = other.at_level(level);
        (a.x.cmp(&b.x), a.y.cmp(&b.y))
    }

    /// True when the two positions name the same point, whatever levels they are written on.
    pub fn same_point(&self, other: &Position) -> bool {
        self.cmp_exact(other) == (Ordering::Equal, Ordering::Equal)
    }
}

impl Delta {
    /// The difference in units of `unit_level`, rounded once to 53 bits (nearest, ties to even) and then scaled by
    /// an exact power of two.
    pub fn to_f64(&self, unit_level: Level) -> [Rel; 2] {
        // A step is 2^-54 of a unit of `self.level`, and a unit of `self.level` is 2^(10 * (U - L)) units of U.
        let exp = -i64::from(UNIT_BITS)
            + i64::from(LEVEL_SHIFT) * (i64::from(unit_level.0) - i64::from(self.level.0));
        [axis_to_rel(&self.dx, exp), axis_to_rel(&self.dy, exp)]
    }
}

fn axis_to_rel(a: &Axis, exp: i64) -> Rel {
    let (neg, mantissa, shift) = a.round_to_53_bits();
    if mantissa == 0 {
        return Rel::Near(0.0);
    }
    // value = mantissa * 2^(shift + exp), with mantissa in [1, 2^53].
    let top = 63 - i64::from(mantissa.leading_zeros()); // floor(log2(mantissa))
    let e = top + shift + exp; // floor(log2(|value|))
    if e >= 1000 {
        return Rel::Far(if neg { -1 } else { 1 });
    }
    if e < -1000 {
        return Rel::Near(0.0);
    }
    // Scale the mantissa into [1, 2] first (exact), then by 2^e (exact, since e is within the normal range).
    let unit = (mantissa as f64) * pow2(-top);
    let v = unit * pow2(e);
    Rel::Near(if neg { -v } else { v })
}

/// 2^k for k in the normal range of an `f64`, built from its bits so that no library function is involved.
fn pow2(k: i64) -> f64 {
    debug_assert!((-1022..=1023).contains(&k));
    f64::from_bits(((k + 1023) as u64) << 52)
}
