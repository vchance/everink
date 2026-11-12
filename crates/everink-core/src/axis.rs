//! One axis of a position: an exact signed integer of steps.
//!
//! The design writes an axis as `cell * 2^64 + frac`. Here it is held as that one integer, with `cell()` and
//! `frac()` as views of its high and low part. A value that fits 128 bits is held inline and costs no
//! allocation; anything larger is a big integer. The form is canonical: a value that fits inline is always
//! held inline, so equality of the representation is equality of the value.

use std::cmp::Ordering;

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{ToPrimitive, Zero};

use crate::FRAC_BITS;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Axis(Repr);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Repr {
    Small(i128),
    /// Never holds a value that fits an `i128`.
    Big(BigInt),
}

impl Axis {
    pub const ZERO: Axis = Axis(Repr::Small(0));

    pub fn from_i128(v: i128) -> Axis {
        Axis(Repr::Small(v))
    }

    /// Canonical form of a big integer.
    pub fn from_big(b: BigInt) -> Axis {
        match b.to_i128() {
            Some(v) => Axis(Repr::Small(v)),
            None => Axis(Repr::Big(b)),
        }
    }

    /// Builds `cell * 2^64 + frac`.
    pub fn from_cell_frac(cell: BigInt, frac: u64) -> Axis {
        Axis::from_big((cell << FRAC_BITS) + BigInt::from(frac))
    }

    pub fn to_big(&self) -> BigInt {
        match &self.0 {
            Repr::Small(v) => BigInt::from(*v),
            Repr::Big(b) => b.clone(),
        }
    }

    /// True when the value is held inline (it fits 128 bits).
    pub fn is_small(&self) -> bool {
        matches!(self.0, Repr::Small(_))
    }

    pub fn is_zero(&self) -> bool {
        matches!(self.0, Repr::Small(0))
    }

    /// -1, 0 or 1.
    pub fn signum(&self) -> i32 {
        match &self.0 {
            Repr::Small(v) => v.signum() as i32,
            Repr::Big(b) => match b.sign() {
                Sign::Minus => -1,
                Sign::NoSign => 0,
                Sign::Plus => 1,
            },
        }
    }

    /// The cell: the value divided by 2^64, rounded toward minus infinity.
    pub fn cell(&self) -> BigInt {
        match &self.0 {
            Repr::Small(v) => BigInt::from(*v >> FRAC_BITS),
            Repr::Big(b) => floor_shr(b, FRAC_BITS as u64),
        }
    }

    /// The offset inside the cell: the value modulo 2^64, never negative.
    pub fn frac(&self) -> u64 {
        match &self.0 {
            // Two's complement: the low 64 bits of a negative value are the value modulo 2^64.
            Repr::Small(v) => *v as u64,
            Repr::Big(b) => {
                let cell = floor_shr(b, FRAC_BITS as u64);
                (b - (cell << FRAC_BITS))
                    .to_u64()
                    .expect("offset is below 2^64")
            }
        }
    }

    /// Multiplies by 2^bits. Exact.
    pub fn shl(&self, bits: u32) -> Axis {
        match &self.0 {
            Repr::Small(0) => Axis::ZERO,
            Repr::Small(v) => {
                if bits < 127 {
                    let shifted = v << bits;
                    if (shifted >> bits) == *v {
                        return Axis(Repr::Small(shifted));
                    }
                }
                // Through `from_big`, so the result is canonical even when it fits 128 bits after all
                // (-1 << 127 does).
                Axis::from_big(BigInt::from(*v) << bits)
            }
            Repr::Big(b) => Axis(Repr::Big(b << bits)),
        }
    }

    /// Divides by 2^bits, rounding toward minus infinity.
    pub fn shr_floor(&self, bits: u32) -> Axis {
        match &self.0 {
            Repr::Small(v) => {
                if bits >= 127 {
                    Axis(Repr::Small(if *v < 0 { -1 } else { 0 }))
                } else {
                    Axis(Repr::Small(v >> bits))
                }
            }
            Repr::Big(b) => Axis::from_big(floor_shr(b, bits as u64)),
        }
    }

    pub fn add(&self, other: &Axis) -> Axis {
        if let (Repr::Small(a), Repr::Small(b)) = (&self.0, &other.0)
            && let Some(s) = a.checked_add(*b)
        {
            return Axis(Repr::Small(s));
        }
        match (&self.0, &other.0) {
            (Repr::Big(a), Repr::Big(b)) => Axis::from_big(a + b),
            (Repr::Big(a), Repr::Small(b)) | (Repr::Small(b), Repr::Big(a)) => {
                Axis::from_big(a + BigInt::from(*b))
            }
            (Repr::Small(a), Repr::Small(b)) => Axis::from_big(BigInt::from(*a) + BigInt::from(*b)),
        }
    }

    pub fn sub(&self, other: &Axis) -> Axis {
        if let (Repr::Small(a), Repr::Small(b)) = (&self.0, &other.0)
            && let Some(s) = a.checked_sub(*b)
        {
            return Axis(Repr::Small(s));
        }
        match (&self.0, &other.0) {
            (Repr::Big(a), Repr::Big(b)) => Axis::from_big(a - b),
            (Repr::Big(a), Repr::Small(b)) => Axis::from_big(a - BigInt::from(*b)),
            (Repr::Small(a), Repr::Big(b)) => Axis::from_big(BigInt::from(*a) - b),
            (Repr::Small(a), Repr::Small(b)) => Axis::from_big(BigInt::from(*a) - BigInt::from(*b)),
        }
    }

    /// The value as `mantissa * 2^exponent` with a 53-bit mantissa, rounded to nearest, ties to even. Returns
    /// `(negative, mantissa, exponent)`. The mantissa is 0 for zero. A value of 53 bits or fewer is returned as
    /// itself with exponent 0; a longer one has a mantissa in `[2^52, 2^53]` (2^53 when rounding carried).
    ///
    /// This is written here and not left to the big-integer crate so that the rounding rule is the same on every
    /// target.
    pub fn round_to_53_bits(&self) -> (bool, u64, i64) {
        match &self.0 {
            Repr::Small(v) => {
                let neg = *v < 0;
                let mag = v.unsigned_abs();
                if mag == 0 {
                    return (false, 0, 0);
                }
                let bits = 128 - mag.leading_zeros();
                if bits <= 53 {
                    return (neg, mag as u64, 0);
                }
                let shift = bits - 53;
                let top = (mag >> shift) as u64;
                let rem = mag & ((1u128 << shift) - 1);
                let half = 1u128 << (shift - 1);
                let up = rem > half || (rem == half && (top & 1) == 1);
                (neg, top + up as u64, shift as i64)
            }
            Repr::Big(b) => {
                let neg = b.sign() == Sign::Minus;
                let mag: &BigUint = b.magnitude();
                let bits = mag.bits();
                let shift = bits - 53;
                let top = (mag >> shift).to_u64().expect("53 bits fit a u64");
                let half_bit = mag.bit(shift - 1);
                let below_half_zero = mag.trailing_zeros().is_none_or(|tz| tz >= shift - 1);
                let up = half_bit && (!below_half_zero || (top & 1) == 1);
                (neg, top + up as u64, shift as i64)
            }
        }
    }
}

/// Division of a big integer by 2^bits, rounding toward minus infinity.
fn floor_shr(b: &BigInt, bits: u64) -> BigInt {
    if b.sign() != Sign::Minus {
        return b >> bits;
    }
    // For a negative value, floor(b / 2^k) = -ceil(|b| / 2^k).
    let mag = b.magnitude();
    let q = mag >> bits;
    let exact = mag.trailing_zeros().is_none_or(|tz| tz >= bits);
    let q = if exact { q } else { q + BigUint::from(1u8) };
    if q.is_zero() {
        BigInt::zero()
    } else {
        BigInt::from_biguint(Sign::Minus, q)
    }
}

impl PartialOrd for Axis {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Axis {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (Repr::Small(a), Repr::Small(b)) => a.cmp(b),
            _ => self.to_big().cmp(&other.to_big()),
        }
    }
}
