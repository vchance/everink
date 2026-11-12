//! The points of a stroke, packed (design section 02, "Strokes").
//!
//! Each point is two whole numbers on the stroke's own lattice and one pressure byte. The bytes hold, for each
//! point, the difference from the point before it (the first from zero) as two zigzag varints, then the pressure.
//! Decoding uses whole numbers only, so it gives the same result on every machine.

use std::sync::Arc;

pub const MAX_STROKE_POINTS: u32 = 20_000;
/// The most bytes one point can take: two differences of up to 33 bits (5 bytes each) and the pressure.
const MAX_POINT_BYTES: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
    /// 0 means the device gave no pressure.
    pub pressure: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointsError {
    TooMany,
    Truncated,
    /// A difference written in more bytes than it needs, or longer than 33 bits.
    NotCanonical,
    /// A point left the 32-bit lattice.
    OutOfRange,
    TrailingBytes,
}

/// Packed points. A value of this type has been checked: it decodes to exactly `count` points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedPoints {
    count: u32,
    bytes: Arc<[u8]>,
}

impl PackedPoints {
    pub fn pack(points: &[Point]) -> Result<PackedPoints, PointsError> {
        if points.len() > MAX_STROKE_POINTS as usize {
            return Err(PointsError::TooMany);
        }
        let mut out = Vec::with_capacity(points.len() * 4);
        let (mut px, mut py) = (0i64, 0i64);
        for p in points {
            put_zigzag(&mut out, i64::from(p.x) - px);
            put_zigzag(&mut out, i64::from(p.y) - py);
            out.push(p.pressure);
            (px, py) = (i64::from(p.x), i64::from(p.y));
        }
        Ok(PackedPoints {
            count: points.len() as u32,
            bytes: out.into(),
        })
    }

    /// Packed points from bytes that came from outside. Refuses anything that does not decode to exactly `count`
    /// points on the lattice. The count is checked against the length first, so a large count cannot cost more
    /// than the bytes that were sent.
    pub fn from_bytes(count: u32, bytes: &[u8]) -> Result<PackedPoints, PointsError> {
        if count > MAX_STROKE_POINTS {
            return Err(PointsError::TooMany);
        }
        if bytes.len() > count as usize * MAX_POINT_BYTES {
            return Err(PointsError::TrailingBytes);
        }
        decode(count, bytes, |_| {})?;
        Ok(PackedPoints {
            count,
            bytes: bytes.into(),
        })
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to_vec(&self) -> Vec<Point> {
        let mut v = Vec::with_capacity(self.count as usize);
        decode(self.count, &self.bytes, |p| v.push(p)).expect("checked when it was built");
        v
    }
}

fn put_zigzag(out: &mut Vec<u8>, d: i64) {
    let mut z = ((d << 1) ^ (d >> 63)) as u64;
    while z >= 0x80 {
        out.push((z as u8) | 0x80);
        z >>= 7;
    }
    out.push(z as u8);
}

fn decode(count: u32, bytes: &[u8], mut each: impl FnMut(Point)) -> Result<(), PointsError> {
    let mut at = 0usize;
    let delta = |at: &mut usize| -> Result<i64, PointsError> {
        let mut z: u64 = 0;
        for i in 0..5 {
            let b = *bytes.get(*at).ok_or(PointsError::Truncated)?;
            *at += 1;
            z |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                // A difference of two 32-bit numbers has at most 33 bits, 34 after zigzag.
                if (i > 0 && b == 0) || z >= 1 << 34 {
                    return Err(PointsError::NotCanonical);
                }
                return Ok(((z >> 1) as i64) ^ -((z & 1) as i64));
            }
        }
        Err(PointsError::NotCanonical)
    };
    let (mut x, mut y) = (0i64, 0i64);
    for _ in 0..count {
        x += delta(&mut at)?;
        y += delta(&mut at)?;
        let pressure = *bytes.get(at).ok_or(PointsError::Truncated)?;
        at += 1;
        let (Ok(xi), Ok(yi)) = (i32::try_from(x), i32::try_from(y)) else {
            return Err(PointsError::OutOfRange);
        };
        each(Point {
            x: xi,
            y: yi,
            pressure,
        });
    }
    if at != bytes.len() {
        return Err(PointsError::TrailingBytes);
    }
    Ok(())
}
