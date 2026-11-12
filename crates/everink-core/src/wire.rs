//! Canonical bytes of a position: one value has exactly one encoding.
//!
//! ```text
//! position := zigzag_varint(level)  axis(x)  axis(y)
//! axis     := varint(len * 2 + sign)  magnitude[len]  frac[8]
//! ```
//!
//! `magnitude` is the absolute value of the cell, big-endian, with no leading zero byte. Cell 0 has `len = 0` and
//! `sign = 0`. `frac` is big-endian. The decoder refuses anything that is not the canonical form.

use num_bigint::{BigInt, Sign};
use num_traits::Zero;

use crate::{Axis, Level, Position};

/// The ceiling on how deep and how far out a position may be. A hostile change cannot make the server build a
/// number larger than this allows, because the cap is checked on the raw bytes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthCap {
    pub max_level: i32,
    pub max_cell_bits: u32,
}

impl DepthCap {
    /// 1,024 levels each way; a cell of at most 12,288 bits. An encoded position is then at most 3,094 bytes.
    pub const DEFAULT: DepthCap = DepthCap {
        max_level: 1024,
        max_cell_bits: 12_288,
    };

    fn max_cell_bytes(&self) -> u64 {
        u64::from(self.max_cell_bits.div_ceil(8))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    /// A number written in more bytes than it needs, a leading zero byte, or a negative zero.
    NotCanonical,
    LevelBeyondCap,
    CellBeyondCap,
    TrailingBytes,
}

impl Position {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(19);
        let zigzag = ((self.level.0 << 1) ^ (self.level.0 >> 31)) as u32;
        put_varint(&mut out, u64::from(zigzag));
        put_axis(&mut out, &self.x);
        put_axis(&mut out, &self.y);
        out
    }

    /// Decodes exactly one position; anything after it is an error.
    pub fn decode(bytes: &[u8], cap: &DepthCap) -> Result<Position, DecodeError> {
        check_cap_bytes(bytes, cap)?;
        let mut r = Reader { bytes, at: 0 };
        let level = r.level()?;
        let x = r.axis()?;
        let y = r.axis()?;
        Ok(Position { level, x, y })
    }
}

/// Checks the shape and the cap on the raw bytes, building no number larger than a `u64`. Run before `decode` on
/// anything from the network.
pub fn check_cap_bytes(bytes: &[u8], cap: &DepthCap) -> Result<(), DecodeError> {
    let mut r = Reader { bytes, at: 0 };
    let level = r.level()?;
    if level.0.unsigned_abs() > cap.max_level.unsigned_abs() {
        return Err(DecodeError::LevelBeyondCap);
    }
    for _ in 0..2 {
        let (len, _negative) = r.axis_header()?;
        if len > cap.max_cell_bytes() {
            return Err(DecodeError::CellBeyondCap);
        }
        // `len` is at most the cap in bytes here, so this cannot overflow.
        let body = r.take(len as usize + 8)?;
        if len > 0 {
            if body[0] == 0 {
                return Err(DecodeError::NotCanonical);
            }
            // The cap is in bits: count the bits of the top byte, not whole bytes.
            let bits = (len - 1) * 8 + u64::from(8 - body[0].leading_zeros());
            if bits > u64::from(cap.max_cell_bits) {
                return Err(DecodeError::CellBeyondCap);
            }
        }
    }
    if r.at != bytes.len() {
        return Err(DecodeError::TrailingBytes);
    }
    Ok(())
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_axis(out: &mut Vec<u8>, a: &Axis) {
    let cell = a.cell();
    if cell.is_zero() {
        put_varint(out, 0);
    } else {
        let (sign, magnitude) = cell.to_bytes_be();
        put_varint(
            out,
            (magnitude.len() as u64) * 2 + u64::from(sign == Sign::Minus),
        );
        out.extend_from_slice(&magnitude);
    }
    out.extend_from_slice(&a.frac().to_be_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(s)
    }

    fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut v: u64 = 0;
        for i in 0..10 {
            let b = self.take(1)?[0];
            let part = u64::from(b & 0x7f);
            if i == 9 && part > 1 {
                return Err(DecodeError::NotCanonical); // would not fit 64 bits
            }
            v |= part << (7 * i);
            if b & 0x80 == 0 {
                if i > 0 && b == 0 {
                    return Err(DecodeError::NotCanonical); // written longer than needed
                }
                return Ok(v);
            }
        }
        Err(DecodeError::NotCanonical)
    }

    fn level(&mut self) -> Result<Level, DecodeError> {
        let z = u32::try_from(self.varint()?).map_err(|_| DecodeError::LevelBeyondCap)?;
        Ok(Level(((z >> 1) as i32) ^ -((z & 1) as i32)))
    }

    /// Returns the magnitude length in bytes and whether the cell is negative.
    fn axis_header(&mut self) -> Result<(u64, bool), DecodeError> {
        let h = self.varint()?;
        let (len, negative) = (h >> 1, h & 1 == 1);
        if len == 0 && negative {
            return Err(DecodeError::NotCanonical); // negative zero
        }
        Ok((len, negative))
    }

    fn axis(&mut self) -> Result<Axis, DecodeError> {
        let (len, negative) = self.axis_header()?;
        let magnitude = self.take(len as usize)?;
        let cell = if len == 0 {
            BigInt::zero()
        } else {
            BigInt::from_bytes_be(if negative { Sign::Minus } else { Sign::Plus }, magnitude)
        };
        let frac = u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"));
        Ok(Axis::from_cell_frac(cell, frac))
    }
}
