//! Spike slice 2b: one thin adapter per candidate text library, so that the same trial runs on each.
//!
//! The design (`docs/design/02-document-model.md`, "Text") holds one small document per text object. Editing
//! clients exchange opaque updates; the server applies each update to a scratch copy, checks its limits, and only
//! then keeps it. `Candidate` is the least a library must do for that.

pub mod candidates;

pub use candidates::{Am, Loro, Yrs};

/// The name every candidate uses for the text inside its document.
pub const TEXT: &str = "t";

pub trait Candidate {
    const NAME: &'static str;
    type Doc;

    /// An empty text document for one client. Two clients that start this way can exchange updates.
    fn new(client: u64) -> Self::Doc;
    /// Inserts at a position counted in characters, and returns the update to send to the others.
    fn insert(doc: &mut Self::Doc, at: usize, text: &str) -> Vec<u8>;
    /// Deletes characters, and returns the update to send to the others.
    fn delete(doc: &mut Self::Doc, at: usize, len: usize) -> Vec<u8>;
    /// Applies an update from outside. Must not panic on any bytes.
    fn apply(doc: &mut Self::Doc, update: &[u8]) -> Result<(), String>;
    fn text(doc: &mut Self::Doc) -> String;
    /// The length in characters.
    fn len(doc: &mut Self::Doc) -> usize;
    /// The whole document as bytes.
    fn snapshot(doc: &mut Self::Doc) -> Vec<u8>;
    fn load(client: u64, snapshot: &[u8]) -> Result<Self::Doc, String>;
    /// A copy to try an update on. What the server's check costs depends mostly on this.
    fn scratch(doc: &mut Self::Doc) -> Self::Doc;

    /// The server's rule: try the update on a scratch copy, check the length limit, and only then keep it.
    fn checked_apply(doc: &mut Self::Doc, update: &[u8], max_chars: usize) -> Result<(), String> {
        let mut scratch = Self::scratch(doc);
        Self::apply(&mut scratch, update)?;
        if Self::len(&mut scratch) > max_chars {
            return Err("the text would be longer than its limit".into());
        }
        Self::apply(doc, update)
    }
}

/// The text typed in the trials: ordinary words, one character per keystroke.
pub fn keystroke(i: usize) -> &'static str {
    const WORDS: &str = "the quick brown fox jumps over the lazy dog ";
    &WORDS[i % WORDS.len()..i % WORDS.len() + 1]
}

/// Types `chars` characters at the end of a new document, one per update.
pub fn typed<C: Candidate>(client: u64, chars: usize) -> (C::Doc, Vec<Vec<u8>>) {
    let mut doc = C::new(client);
    let updates = (0..chars)
        .map(|i| C::insert(&mut doc, i, keystroke(i)))
        .collect();
    (doc, updates)
}

/// SplitMix64: a small deterministic random source, so every run is the same.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}
