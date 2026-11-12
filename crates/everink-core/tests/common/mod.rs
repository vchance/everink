//! Shared helpers for the tests: a small deterministic random source and position generators.
#![allow(dead_code)]

use everink_core::{Axis, Level, Position};
use num_bigint::{BigInt, Sign};

/// SplitMix64. Deterministic, so a failing case can be reproduced from its seed, and the golden file is the same
/// on every machine.
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

    /// A signed integer with at most `bits` bits of magnitude.
    pub fn big(&mut self, bits: u64) -> BigInt {
        if bits == 0 {
            return BigInt::from(0);
        }
        let bytes = bits.div_ceil(8) as usize;
        let mut v: Vec<u8> = (0..bytes).map(|_| self.next() as u8).collect();
        let extra = (bytes as u64) * 8 - bits;
        v[0] &= 0xff >> extra;
        let sign = if self.next() & 1 == 1 {
            Sign::Minus
        } else {
            Sign::Plus
        };
        BigInt::from_bytes_be(sign, &v)
    }

    /// An axis whose cell has at most `cell_bits` bits.
    pub fn axis(&mut self, cell_bits: u64) -> Axis {
        let bits = self.below(cell_bits + 1);
        Axis::from_cell_frac(self.big(bits), self.next())
    }

    /// A position on a level in `[-max_level, max_level]`.
    pub fn position(&mut self, max_level: i32, cell_bits: u64) -> Position {
        let level = self.below(2 * max_level as u64 + 1) as i32 - max_level;
        Position {
            level: Level(level),
            x: self.axis(cell_bits),
            y: self.axis(cell_bits),
        }
    }
}

/// A position `depth` levels down that is near the level-0 origin: the kind a person reaches by zooming in.
pub fn deep_position(rng: &mut Rng, depth: i32) -> Position {
    let near_origin = Position {
        level: Level(0),
        x: Axis::from_i128((rng.next() >> 1) as i128),
        y: Axis::from_i128((rng.next() >> 1) as i128),
    };
    let mut p = near_origin.at_level(Level(depth));
    // Fill in the low bits so the position is not a round number at its own level.
    p.x = p.x.add(&Axis::from_i128(rng.next() as i128));
    p.y = p.y.add(&Axis::from_i128(rng.next() as i128));
    p
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
