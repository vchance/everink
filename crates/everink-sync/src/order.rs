//! Order keys: where an object sits among its siblings (design section 02, "Order").
//!
//! A key is a string of bytes compared as unsigned bytes, a prefix sorting first. Between any two keys there is
//! always another, so an object can be put between two neighbours without renumbering anything. A key never ends
//! in a zero byte, which is what keeps room below every key. Siblings sort by `(key, object id)`, so two objects
//! that were given the same key still have one order everywhere.

use crate::rng::SplitMix64;

pub const MAX_ORDER_KEY_BYTES: usize = 64;
/// Past this length the design has the server rewrite a sibling list's keys. The prototype does not; it reports
/// the lengths it sees.
pub const REBALANCE_KEY_BYTES: usize = 24;
const INLINE: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderError {
    Empty,
    TooLong,
    EndsInZero,
    /// `between` was asked for a key between `a` and `b` with `a` not below `b`.
    NotAscending,
}

/// A key of up to 14 bytes is held inline and costs no allocation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct OrderKey(Repr);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    /// Bytes past `len` are zero, so equal keys are equal values.
    Inline { len: u8, bytes: [u8; INLINE] },
    /// Never holds a key that fits inline.
    Heap(Box<[u8]>),
}

impl OrderKey {
    pub fn new(bytes: &[u8]) -> Result<OrderKey, OrderError> {
        match bytes.last() {
            None => return Err(OrderError::Empty),
            Some(0) => return Err(OrderError::EndsInZero),
            Some(_) => {}
        }
        if bytes.len() > MAX_ORDER_KEY_BYTES {
            return Err(OrderError::TooLong);
        }
        Ok(OrderKey(if bytes.len() <= INLINE {
            let mut b = [0u8; INLINE];
            b[..bytes.len()].copy_from_slice(bytes);
            Repr::Inline {
                len: bytes.len() as u8,
                bytes: b,
            }
        } else {
            Repr::Heap(bytes.into())
        }))
    }

    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            Repr::Inline { len, bytes } => &bytes[..*len as usize],
            Repr::Heap(b) => b,
        }
    }

    /// Bytes this key holds outside the object.
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            Repr::Inline { .. } => 0,
            Repr::Heap(b) => b.len(),
        }
    }

    /// A key strictly between `a` and `b`. `None` for `a` means "below everything" and for `b` "above
    /// everything". The keys are read as digits in base 256. At the first place where at least two digit values
    /// lie strictly between the two, one is picked at random from the middle third and the key ends there.
    /// Otherwise `a`'s digit is kept and the search goes one place further.
    pub fn between(
        a: Option<&OrderKey>,
        b: Option<&OrderKey>,
        rng: &mut SplitMix64,
    ) -> Result<OrderKey, OrderError> {
        let a = a.map_or(&[][..], OrderKey::as_bytes);
        let mut b = b.map(OrderKey::as_bytes);
        if let Some(b) = b
            && a >= b
        {
            return Err(OrderError::NotAscending);
        }
        let mut out: Vec<u8> = Vec::with_capacity(a.len() + 1);
        for i in 0.. {
            if i >= MAX_ORDER_KEY_BYTES {
                return Err(OrderError::TooLong);
            }
            let lo = u32::from(a.get(i).copied().unwrap_or(0));
            // Past the end of `b`, or once the key is already below `b`, the upper limit is one past the last
            // digit value.
            let hi = b.and_then(|b| b.get(i)).map_or(256, |d| u32::from(*d));
            // `a < b` makes `hi >= lo`. Three apart leaves two digit values strictly between.
            if hi >= lo + 3 {
                let free = hi - lo - 1;
                let third = free / 3;
                let pick = lo + 1 + third + rng.below(u64::from(free - 2 * third)) as u32;
                out.push(pick as u8);
                return OrderKey::new(&out);
            }
            out.push(lo as u8);
            if hi != lo {
                b = None; // the key is now below `b` whatever follows
            }
        }
        unreachable!("the loop returns")
    }
}

/// Keys that start with one of these bytes are counters: the byte says how many bytes of whole number follow
/// (`0xF8` one byte, up to `0xFE` seven), and a longer number sorts above every shorter one. `after` counts up in
/// them. `0x07` down to `0x01` are the same for `before`, counting down.
const FRONT_FIRST: u8 = 0xF8;
const FRONT_LAST: u8 = 0xFE;
const BACK_FIRST: u8 = 0x07;
const BACK_LAST: u8 = 0x01;

impl OrderKey {
    /// A key above `a`, for putting an object in front of every sibling. `None` gives the first key of an empty
    /// list.
    ///
    /// `between(a, None)` would do, but each call takes a share of the space that is left, so a key grows by a
    /// byte about every seven calls and reaches the length limit after a few hundred. New objects go in front
    /// far more often than anywhere else. This counts instead: 100,000 objects put in front one after another
    /// have keys of at most four bytes. Only the server calls it, one change at a time, so no two calls can pick
    /// the same key and it needs no randomness of its own.
    pub fn after(a: Option<&OrderKey>, rng: &mut SplitMix64) -> Result<OrderKey, OrderError> {
        let Some(a) = a else {
            return OrderKey::new(&[0x80]);
        };
        let bytes = a.as_bytes();
        let head = bytes[0];
        if head < FRONT_FIRST {
            return OrderKey::new(&[FRONT_FIRST, 1]);
        }
        if head > FRONT_LAST {
            return OrderKey::between(Some(a), None, rng);
        }
        let len = usize::from(head - FRONT_FIRST) + 1;
        let number = counter(bytes, len);
        // Never a number that ends in a zero byte: a key may not end in one.
        let mut next = number + 1;
        if next & 0xff == 0 {
            next += 1;
        }
        if next >> (8 * len) != 0 {
            if head == FRONT_LAST {
                return OrderKey::between(Some(a), None, rng);
            }
            let mut out = vec![0u8; len + 2];
            out[0] = head + 1;
            out[len + 1] = 1;
            return OrderKey::new(&out);
        }
        let mut out = vec![head];
        out.extend_from_slice(&next.to_be_bytes()[8 - len..]);
        OrderKey::new(&out)
    }

    /// A key below `a`, for putting an object behind every sibling. The mirror of `after`.
    pub fn before(a: Option<&OrderKey>, rng: &mut SplitMix64) -> Result<OrderKey, OrderError> {
        let Some(a) = a else {
            return OrderKey::new(&[0x80]);
        };
        let bytes = a.as_bytes();
        let head = bytes[0];
        if head > BACK_FIRST {
            return OrderKey::new(&[BACK_FIRST, 0xff]);
        }
        if head < BACK_LAST {
            return OrderKey::between(None, Some(a), rng);
        }
        let len = usize::from(BACK_FIRST - head) + 1;
        let number = counter(bytes, len);
        // One down, and one more if that ends in a zero byte. Running out means the next longer number.
        let prev = match number.checked_sub(1) {
            Some(p) if p & 0xff != 0 => Some(p),
            Some(p) => p.checked_sub(1),
            None => None,
        };
        let Some(prev) = prev else {
            if head == BACK_LAST {
                return OrderKey::between(None, Some(a), rng);
            }
            let mut out = vec![0xffu8; len + 2];
            out[0] = head - 1;
            return OrderKey::new(&out);
        };
        let mut out = vec![head];
        out.extend_from_slice(&prev.to_be_bytes()[8 - len..]);
        OrderKey::new(&out)
    }
}

/// The whole number in the `len` bytes after the first; a key shorter than that counts as ending in zeros.
fn counter(bytes: &[u8], len: usize) -> u64 {
    (0..len).fold(0u64, |n, i| {
        (n << 8) | u64::from(bytes.get(1 + i).copied().unwrap_or(0))
    })
}

impl std::fmt::Debug for OrderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OrderKey(")?;
        for b in self.as_bytes() {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}
