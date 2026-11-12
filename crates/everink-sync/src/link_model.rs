//! A model of one connection with a bulk load on it, in made-up time, to see what the pacing rule does to cursor
//! delay without a network. The same numbers come out on every run and every machine.
//!
//! The model is the simplest that has the effect in question: the connection sends a fixed number of bytes per
//! second, in the order they were handed to it. A cursor message is delivered when its last byte has been sent.
//! Nothing is lost, nothing is reordered, and the far end reads at once. A real connection adds its own queues
//! and its own changes of speed; `examples/paced_link.rs` runs the same pacer over real sockets.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::pacing::{Class, Pacer, Rule};

/// The bytes of a frame's header on the wire: a class and a length.
pub const FRAME_HEADER: usize = 5;
/// A cursor message: 28 bytes at depth 0 (design section 04).
pub const CURSOR_BYTES: usize = 28;
/// Cursors are sent this often: 30 a second.
pub const CURSOR_EVERY: Duration = Duration::from_micros(33_333);
const STEP: Duration = Duration::from_millis(1);

/// How the bytes not yet sent are counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queues {
    /// One queue: the writer knows every byte that has not left, and keeps all of them under the limit. This is
    /// the rule as written.
    One,
    /// Two queues, as with a real socket: the operating system holds up to the limit of unsent bytes
    /// (`TCP_NOTSENT_LOWAT`) and says only "there is room" or "there is not"; the writer hands over its next
    /// chunk whenever there is room. Up to one chunk more than the limit can then be waiting.
    Two,
}

#[derive(Clone, Debug)]
pub struct Report {
    /// How long each cursor message waited, in the order they were handed over. A message still on its way at
    /// the end counts with the time it had waited by then, so a connection that never delivers does not look fast.
    pub delays: Vec<Duration>,
    /// How many of them were still on their way at the end.
    pub undelivered: usize,
    /// Cursor messages that were replaced by a newer one before they could be sent.
    pub replaced: usize,
    /// The limit on unsent bytes at the end.
    pub limit: usize,
    /// When the limit first came within a tenth of its final value, from the start of the load.
    pub settled_after: Option<Duration>,
    /// The most bytes that were ever handed over and unsent.
    pub most_unsent: usize,
    /// Bulk bytes delivered.
    pub bulk_sent: usize,
}

impl Report {
    /// Median, 95th percentile and maximum, in milliseconds.
    pub fn delay_ms(&self) -> (f64, f64, f64) {
        let mut v: Vec<f64> = self.delays.iter().map(|d| d.as_secs_f64() * 1e3).collect();
        if v.is_empty() {
            return (f64::NAN, f64::NAN, f64::NAN);
        }
        v.sort_by(|a, b| a.partial_cmp(b).expect("delays are numbers"));
        (
            v[v.len() / 2],
            v[(v.len() * 95 / 100).min(v.len() - 1)],
            v[v.len() - 1],
        )
    }
}

/// Runs a bulk load of `bulk_bytes` over a connection of `bits_per_second` for `duration`, with one other
/// person's cursor arriving 30 times a second, and reports how long the cursor messages waited.
pub fn run(
    rule: Rule,
    first_limit: usize,
    queues: Queues,
    bits_per_second: f64,
    bulk_bytes: usize,
    duration: Duration,
) -> Report {
    let start = Instant::now();
    let mut pacer = Pacer::new(rule).with_first_limit(first_limit);
    pacer.bulk(&vec![0u8; bulk_bytes]);

    let bytes_per_step = bits_per_second / 8.0 * STEP.as_secs_f64();
    // Bytes handed to the connection and not yet sent, and for each cursor frame among them, the count of bytes
    // handed over when its last byte was, with the time it was made.
    let mut unsent: usize = 0;
    let mut handed: usize = 0;
    let mut sent: usize = 0;
    let mut cursors: VecDeque<(usize, Duration)> = VecDeque::new();
    let mut budget = 0.0f64;
    let mut next_cursor = Duration::ZERO;
    let mut made = 0usize;
    let mut report = Report {
        delays: Vec::new(),
        undelivered: 0,
        replaced: 0,
        limit: 0,
        settled_after: None,
        most_unsent: 0,
        bulk_sent: 0,
    };
    let mut limits: Vec<(Duration, usize)> = Vec::new();
    let mut bulk_handed = 0usize;

    let mut at = Duration::ZERO;
    while at < duration {
        // Someone's cursor moves.
        while next_cursor <= at {
            let mut message = vec![0u8; CURSOR_BYTES];
            message[..8].copy_from_slice(&(next_cursor.as_micros() as u64).to_le_bytes());
            pacer.presence(1, message);
            made += 1;
            next_cursor += CURSOR_EVERY;
        }
        // The writer hands over what the rule allows.
        loop {
            let counted = match queues {
                Queues::One => unsent,
                // The socket says there is room; the writer does not know how much is in it.
                Queues::Two if unsent < pacer.limit() => 0,
                Queues::Two => usize::MAX,
            };
            let Some((class, bytes)) = pacer.next(counted) else {
                break;
            };
            let frame = FRAME_HEADER + bytes.len();
            unsent += frame;
            handed += frame;
            match class {
                Class::Presence => {
                    let made_at = u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes"));
                    cursors.push_back((handed, Duration::from_micros(made_at)));
                }
                Class::Bulk => bulk_handed += bytes.len(),
                _ => {}
            }
        }
        report.most_unsent = report.most_unsent.max(unsent);
        // The connection sends for one step.
        at += STEP;
        budget += bytes_per_step;
        let go = (budget as usize).min(unsent);
        budget -= go as f64;
        unsent -= go;
        sent += go;
        if unsent == 0 {
            budget = 0.0; // an idle connection does not save up
        }
        pacer.sent(start + at, go, unsent > 0);
        while let Some(&(end, made_at)) = cursors.front() {
            if end > sent {
                break;
            }
            report.delays.push(at - made_at);
            cursors.pop_front();
        }
        limits.push((at, pacer.limit()));
    }
    // Every cursor message was delivered, is on its way, is still waiting to be handed over, or was replaced.
    report.replaced = made - report.delays.len() - cursors.len() - pacer.presence_waiting();
    report.undelivered = cursors.len();
    for (_, made_at) in cursors {
        report.delays.push(at - made_at);
    }
    report.limit = pacer.limit();
    report.bulk_sent = bulk_handed.saturating_sub(unsent);
    let last = report.limit as f64;
    report.settled_after = limits
        .iter()
        .find(|(_, l)| (*l as f64 - last).abs() <= last / 10.0)
        .map(|(t, _)| *t);
    report
}
