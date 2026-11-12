//! A small deterministic random source (SplitMix64).
//!
//! Order keys pick a random digit so that two people inserting at the same place seldom pick the same key. That
//! needs no secrecy, and a fixed seed makes every test and measurement repeatable.

#[derive(Clone, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`. `n` must not be zero.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
