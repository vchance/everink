//! What a connection sends next (design section 03, "Flow control", and section 04, "One connection").
//!
//! Cursors and bulk data share one connection. A cursor message written to the connection waits behind every
//! byte that was written before it and has not left yet, so the way to keep cursors prompt is to never hand the
//! connection much more than it can send soon. The rule:
//!
//! > Keep the connection's unsent bytes below what it drains in about 20 ms, clamped to between 4 KiB and
//! > 64 KiB, and write bulk data in chunks no larger than that limit and never over 16 KiB.
//!
//! `Pacer` holds the four outbound queues and answers one question: given how many bytes are handed over and
//! not yet sent, what is written next? It has no clock and no socket of its own; the caller tells it what the
//! connection accepted and when, which makes it testable with made-up time.
//!
//! - **Presence** (cursors): only the newest message of each sender is kept, and it goes first, whatever is
//!   unsent.
//! - **Control** (acks, rejects, notices): next, also whatever is unsent. These are small and rare.
//! - **Apply** (accepted changes) and **Bulk** (objects coming into view) are byte streams, cut into chunks.
//!   A chunk is taken only if the unsent bytes plus the chunk stay under the limit. Accepted changes go before
//!   bulk, which is the lowest class; after eight chunks of them in a row one chunk of bulk goes, so a load is
//!   never starved.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub const MIN_LIMIT: usize = 4 * 1024;
pub const MAX_LIMIT: usize = 64 * 1024;
pub const MAX_CHUNK: usize = 16 * 1024;
/// The limit before the first estimate of the drain rate.
pub const FIRST_LIMIT: usize = 16 * 1024;
/// A window of the drain-rate estimate is at least this long.
pub const ESTIMATE_WINDOW: Duration = Duration::from_millis(50);
/// After this many chunks of accepted changes in a row, one chunk of bulk goes.
const TURN: u32 = 8;
/// A chunk smaller than this is not worth a frame, unless it is all that is left.
const SMALLEST_CHUNK: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Presence,
    Control,
    Apply,
    Bulk,
}

/// How the limit on unsent bytes is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rule {
    /// What the connection drains in this long, clamped to `[MIN_LIMIT, MAX_LIMIT]`. The design's rule is 20 ms.
    Drain(Duration),
    /// A fixed number of bytes, for comparison: what a connection does with no pacing at all.
    Fixed(usize),
}

pub struct Pacer {
    rule: Rule,
    first_limit: usize,
    /// The newest presence message of each sender, and the order in which senders are served.
    presence: HashMap<u64, Vec<u8>>,
    presence_order: VecDeque<u64>,
    control: VecDeque<Vec<u8>>,
    apply: VecDeque<u8>,
    bulk: VecDeque<u8>,
    /// How many chunks of accepted changes have gone since the last chunk of bulk.
    applies_in_a_row: u32,
    /// Bytes per second, once there has been a full window with a backlog.
    rate: Option<f64>,
    window: Option<(Instant, usize)>,
}

impl Pacer {
    pub fn new(rule: Rule) -> Pacer {
        Pacer {
            rule,
            first_limit: FIRST_LIMIT,
            presence: HashMap::new(),
            presence_order: VecDeque::new(),
            control: VecDeque::new(),
            apply: VecDeque::new(),
            bulk: VecDeque::new(),
            applies_in_a_row: 0,
            rate: None,
            window: None,
        }
    }

    /// Changes the limit used before the first estimate of the drain rate (the design's value is 16 KiB).
    pub fn with_first_limit(mut self, bytes: usize) -> Pacer {
        self.first_limit = bytes;
        self
    }

    /// A presence message from `sender`. It replaces an older one of the same sender that has not gone out.
    pub fn presence(&mut self, sender: u64, message: Vec<u8>) {
        if self.presence.insert(sender, message).is_none() {
            self.presence_order.push_back(sender);
        }
    }

    pub fn control(&mut self, message: Vec<u8>) {
        self.control.push_back(message);
    }

    /// Bytes for the stream of accepted changes. The receiver puts the chunks back together.
    pub fn apply(&mut self, bytes: &[u8]) {
        self.apply.extend(bytes);
    }

    /// Bytes for the bulk stream.
    pub fn bulk(&mut self, bytes: &[u8]) {
        self.bulk.extend(bytes);
    }

    /// Presence messages waiting to be handed over: at most one per sender.
    pub fn presence_waiting(&self) -> usize {
        self.presence.len()
    }

    /// Bytes waiting in the two streams.
    pub fn backlog(&self) -> usize {
        self.apply.len() + self.bulk.len()
    }

    pub fn is_empty(&self) -> bool {
        self.presence.is_empty() && self.control.is_empty() && self.backlog() == 0
    }

    /// The estimate of what the connection sends, in bytes per second.
    pub fn rate(&self) -> Option<f64> {
        self.rate
    }

    /// The most bytes that may be handed over and unsent.
    pub fn limit(&self) -> usize {
        match self.rule {
            Rule::Fixed(n) => n,
            Rule::Drain(target) => match self.rate {
                None => self.first_limit,
                Some(rate) => ((rate * target.as_secs_f64()) as usize).clamp(MIN_LIMIT, MAX_LIMIT),
            },
        }
    }

    /// What to hand to the connection next, when `unsent` bytes are handed over and have not left. `None` means
    /// nothing may go yet (or there is nothing).
    pub fn next(&mut self, unsent: usize) -> Option<(Class, Vec<u8>)> {
        if let Some(sender) = self.presence_order.pop_front() {
            let message = self
                .presence
                .remove(&sender)
                .expect("a sender in the order has a message");
            return Some((Class::Presence, message));
        }
        if let Some(message) = self.control.pop_front() {
            return Some((Class::Control, message));
        }
        let limit = self.limit();
        // The chunk must leave the unsent bytes under the limit, and is never over 16 KiB.
        let room = limit
            .saturating_sub(unsent)
            .saturating_sub(1)
            .min(MAX_CHUNK);
        // Accepted changes go before bulk. After eight chunks of them in a row, one chunk of bulk goes, so a load
        // is never starved. If the stream whose turn it is has no chunk that fits, the other may have one.
        let bulk_turn = self.applies_in_a_row >= TURN && !self.bulk.is_empty();
        let order = if bulk_turn {
            [Class::Bulk, Class::Apply]
        } else {
            [Class::Apply, Class::Bulk]
        };
        let mut chosen = None;
        for stream in order {
            let waiting = self.stream(stream).len();
            let take = waiting.min(room);
            if take > 0 && (take >= SMALLEST_CHUNK || take == waiting) {
                chosen = Some((stream, take));
                break;
            }
        }
        let (stream, take) = chosen?;
        if stream == Class::Apply {
            self.applies_in_a_row += 1;
        } else {
            self.applies_in_a_row = 0;
        }
        let queue = match stream {
            Class::Apply => &mut self.apply,
            _ => &mut self.bulk,
        };
        Some((stream, queue.drain(..take).collect()))
    }

    fn stream(&self, class: Class) -> &VecDeque<u8> {
        match class {
            Class::Apply => &self.apply,
            _ => &self.bulk,
        }
    }

    /// Tells the pacer that the connection took `bytes` more at `now`. `backlogged` says whether there were still
    /// bytes it had not taken: only then does how much it took say how fast it is. The estimate is the average of
    /// windows of at least 50 ms, the newest counting for a quarter.
    pub fn sent(&mut self, now: Instant, bytes: usize, backlogged: bool) {
        if !backlogged {
            // An idle connection says nothing about its speed; the window starts again.
            self.window = None;
            return;
        }
        let (start, total) = self.window.get_or_insert((now, 0));
        *total += bytes;
        let elapsed = now.duration_since(*start);
        if elapsed >= ESTIMATE_WINDOW {
            let newest = *total as f64 / elapsed.as_secs_f64();
            self.rate = Some(match self.rate {
                None => newest,
                Some(old) => old * 0.75 + newest * 0.25,
            });
            self.window = Some((now, 0));
        }
    }
}
