//! Tests for the bytes of a change on the wire, the pacing rule, and a server with two clients that talk only in
//! those bytes.

mod common;

use std::time::{Duration, Instant};

use common::{CAP, World, client_id};
use everink_sync::link_model::{self, FRAME_HEADER, Queues};
use everink_sync::pacing::{Class, FIRST_LIMIT, MAX_CHUNK, MAX_LIMIT, MIN_LIMIT, Pacer, Rule};
use everink_sync::wire::{decode_applied, decode_change, encode_applied, encode_change};
use everink_sync::{DecodeError, Object, Outcome, Replica, Room, UserId};

const MS20: Rule = Rule::Drain(Duration::from_millis(20));

// ---------------------------------------------------------------------------------------------------------------
// Bytes on the wire
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn changes_and_their_answers_survive_the_wire_unchanged() {
    let mut w = World::new(51, 3);
    let mut sent = 0;
    for _ in 0..3_000 {
        let client = w.rng.below(3) as usize;
        let change = w.random_change(client);
        // Every change a client makes, accepted or not.
        let bytes = encode_change(&change);
        assert_eq!(decode_change(&bytes, &CAP).unwrap(), change);
        if let Outcome::Accepted(applied) = w.submit(client, &change) {
            let bytes = encode_applied(&applied);
            assert_eq!(decode_applied(&bytes, &CAP).unwrap(), applied);
        }
        sent += 1;
    }
    assert!(
        w.tally.accepted > 800 && sent == 3_000,
        "accepted {}",
        w.tally.accepted
    );
}

#[test]
fn damaged_wire_bytes_are_refused_or_are_another_valid_message() {
    let mut w = World::new(52, 2);
    let (mut refused, mut accepted) = (0, 0);
    for _ in 0..150 {
        let change = w.random_change(0);
        let bytes = encode_change(&change);
        for cut in 0..bytes.len() {
            assert!(decode_change(&bytes[..cut], &CAP).is_err());
        }
        for _ in 0..40 {
            let mut m = bytes.clone();
            let i = w.rng.below(m.len() as u64) as usize;
            m[i] ^= 1 << w.rng.below(8);
            match decode_change(&m, &CAP) {
                // No two encodings of one value: what decodes, encodes to exactly the bytes it came from.
                Ok(other) => {
                    assert_eq!(encode_change(&other), m);
                    accepted += 1;
                }
                Err(_) => refused += 1,
            }
        }
        if let Outcome::Accepted(applied) = w.submit(0, &change) {
            let bytes = encode_applied(&applied);
            for cut in 0..bytes.len() {
                assert!(decode_applied(&bytes[..cut], &CAP).is_err());
            }
            for _ in 0..40 {
                let mut m = bytes.clone();
                let i = w.rng.below(m.len() as u64) as usize;
                m[i] ^= 1 << w.rng.below(8);
                match decode_applied(&m, &CAP) {
                    Ok(other) => {
                        assert_eq!(encode_applied(&other), m);
                        accepted += 1;
                    }
                    Err(_) => refused += 1,
                }
            }
        }
    }
    assert!(
        refused > 1_000 && accepted > 1_000,
        "refused {refused}, accepted {accepted}"
    );

    // A count of operations above the limit is refused from the count alone, before anything is set aside.
    let too_many = [1, 0, 0xe9, 0x07]; // id 1, observed 0, 1,001 operations, and no operations
    assert_eq!(
        decode_change(&too_many, &CAP),
        Err(DecodeError::Invalid(
            "more operations than a change may hold"
        ))
    );
    let mut trailing = encode_change(&w.random_change(1));
    trailing.push(0);
    assert_eq!(
        decode_change(&trailing, &CAP),
        Err(DecodeError::TrailingBytes)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// A server and two clients that talk in bytes
// ---------------------------------------------------------------------------------------------------------------

/// What one connection carries to a client: frames of `class, length, payload`, cut by the pacer.
struct Line {
    pacer: Pacer,
    wire: Vec<u8>,
}

impl Line {
    /// A line on a slow connection: the limit stays at its floor, so chunks are small.
    fn new() -> Line {
        Line {
            pacer: Pacer::new(Rule::Fixed(MIN_LIMIT)),
            wire: Vec::new(),
        }
    }

    /// Sends up to `frames` frames, as a connection that takes a little at a time.
    fn send(&mut self, frames: usize) {
        for _ in 0..frames {
            let Some((class, bytes)) = self.pacer.next(0) else {
                return;
            };
            self.wire.push(match class {
                Class::Presence => 0,
                Class::Control => 1,
                Class::Apply => 2,
                Class::Bulk => 3,
            });
            self.wire
                .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            self.wire.extend_from_slice(&bytes);
        }
    }
}

/// A client's end: puts the two streams back together and merges what they hold, in the order it arrives.
struct Reader {
    replica: Replica,
    apply: Vec<u8>,
    bulk: Vec<u8>,
    presence: Vec<Vec<u8>>,
    /// Frames of bulk that arrived after a frame of changes had: the two streams really were mixed.
    bulk_after_apply: usize,
    seen_apply_since_join: bool,
}

impl Reader {
    fn new(n: u8) -> Reader {
        Reader {
            replica: Replica::new(client_id(n), UserId(u64::from(n))),
            apply: Vec::new(),
            bulk: Vec::new(),
            presence: Vec::new(),
            bulk_after_apply: 0,
            seen_apply_since_join: false,
        }
    }

    fn read(&mut self, wire: &mut Vec<u8>) {
        let mut at = 0;
        while wire.len() - at >= FRAME_HEADER {
            let len = u32::from_le_bytes(wire[at + 1..at + 5].try_into().unwrap()) as usize;
            let payload = &wire[at + FRAME_HEADER..at + FRAME_HEADER + len];
            match wire[at] {
                0 => self.presence.push(payload.to_vec()),
                2 => {
                    self.apply.extend_from_slice(payload);
                    self.seen_apply_since_join = true;
                    self.records(false);
                }
                3 => {
                    self.bulk.extend_from_slice(payload);
                    self.bulk_after_apply += usize::from(self.seen_apply_since_join);
                    self.records(true);
                }
                other => panic!("an unknown class {other}"),
            }
            at += FRAME_HEADER + len;
        }
        wire.drain(..at);
    }

    /// Takes the whole records off the front of one stream. A record is a 4-byte length, then its bytes. In the
    /// bulk stream an empty record says that every object of the view has been sent.
    fn records(&mut self, bulk: bool) {
        let stream = if bulk {
            &mut self.bulk
        } else {
            &mut self.apply
        };
        let mut at = 0;
        while stream.len() - at >= 4 {
            let len = u32::from_le_bytes(stream[at..at + 4].try_into().unwrap()) as usize;
            if stream.len() - at < 4 + len {
                break;
            }
            let record = &stream[at + 4..at + 4 + len];
            if !bulk {
                self.replica.apply(&decode_applied(record, &CAP).unwrap());
            } else if record.is_empty() {
                self.replica.end_fill();
            } else {
                self.replica.enter(&Object::decode(record, &CAP).unwrap());
            }
            at += 4 + len;
        }
        stream.drain(..at);
    }
}

fn record(bytes: &[u8]) -> Vec<u8> {
    let mut r = (bytes.len() as u32).to_le_bytes().to_vec();
    r.extend_from_slice(bytes);
    r
}

/// The first client draws and edits. The second joins late: it is sent the room as it was when it joined (the
/// bulk stream) while changes keep arriving (the apply stream) and the first client's cursor is passed on
/// (presence). Everything the server and the clients exchange is bytes. The connection to the second client is
/// slow, so the room takes many rounds to arrive and changes overtake the objects they write to.
///
/// Returns the second client, the server's room, and what the first client holds compared with the room.
fn late_joiner(keep_writes_during_the_fill: bool) -> (Reader, Room) {
    let mut w = World::new(61, 1);
    // The same seed as the world's room, so that both pick the same order keys.
    let mut room = Room::new(CAP, 61 ^ 0xabcd);
    let (mut first, mut second) = (Reader::new(0), Reader::new(1));
    let (mut to_first, mut to_second) = (Line::new(), Line::new());
    let mut joined = false;
    for round in 0..3_000 {
        // The world's own room is only there to make changes that fit the state; the server is `room`.
        let change = w.random_change(0);
        w.submit(0, &change);
        first.replica.local(change.clone());
        let from_client = encode_change(&change);

        // The server decodes the bytes before it looks at them.
        let decoded = decode_change(&from_client, &CAP).unwrap();
        match room.submit(client_id(0), UserId(0), &decoded) {
            Outcome::Accepted(applied) => {
                let bytes = record(&encode_applied(&applied));
                to_first.pacer.apply(&bytes);
                if joined {
                    to_second.pacer.apply(&bytes);
                }
            }
            Outcome::Rejected(_) => first.replica.rejected(change.id),
            Outcome::Duplicate { .. } => panic!("a duplicate"),
        }
        if round == 1_000 {
            // The second client joins: every object as it is now goes into the bulk stream, then the mark that
            // says the view is complete. Changes accepted from here on go to it as well.
            joined = true;
            if keep_writes_during_the_fill {
                second.replica.begin_fill();
            }
            for object in room.objects() {
                to_second.pacer.bulk(&record(&object.encode()));
            }
            to_second.pacer.bulk(&record(&[]));
        }
        if joined {
            to_second
                .pacer
                .presence(0, (round as u32).to_le_bytes().to_vec());
        }
        to_first.send(usize::MAX);
        first.read(&mut to_first.wire);
        // The slow connection: two frames a round, of which the cursor takes one.
        to_second.send(2);
        second.read(&mut to_second.wire);
    }
    to_second.send(usize::MAX);
    second.read(&mut to_second.wire);
    for (line, reader) in [(&to_first, &first), (&to_second, &second)] {
        assert!(line.wire.is_empty() && reader.apply.is_empty() && reader.bulk.is_empty());
    }
    assert!(w.tally.accepted > 1_000 && room.len() > 500);
    assert_eq!(
        room.encode_objects(),
        w.room.encode_objects(),
        "the two servers differ"
    );
    assert_eq!(first.replica.encode_objects(), room.encode_objects());
    assert!(first.replica.pending().is_empty());
    (second, room)
}

#[test]
fn a_late_joiner_ends_with_the_same_objects_when_changes_overtake_the_room() {
    let (second, room) = late_joiner(true);
    // The two streams really were mixed: most of the room arrived after changes had begun to.
    assert!(
        second.bulk_after_apply > 5,
        "{} frames",
        second.bulk_after_apply
    );
    assert_eq!(second.replica.encode_objects(), room.encode_objects());
    assert_eq!(
        second.replica.kept_writes(),
        0,
        "the fill ended and kept nothing"
    );
    // Presence is the newest only: the second client saw fewer cursor messages than were made, in order, and
    // the last one it saw is the last one made.
    let seen: Vec<u32> = second
        .presence
        .iter()
        .map(|p| u32::from_le_bytes(p[..4].try_into().unwrap()))
        .collect();
    assert!(seen.len() <= 2_000 && seen.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(seen.last(), Some(&2_999));
}

#[test]
fn without_keeping_writes_during_the_fill_a_late_joiner_loses_them() {
    // The same run with a client that drops a write to an object it does not hold yet, as a copy does outside
    // a fill. A change that overtook its object is lost: the object then arrives as it was when the client
    // joined. This is why `Replica::begin_fill` exists, and what the design has to say about a view fill.
    let (second, room) = late_joiner(false);
    assert!(second.bulk_after_apply > 5);
    assert_eq!(second.replica.len(), room.len(), "every object arrived");
    assert_ne!(second.replica.encode_objects(), room.encode_objects());
}

// ---------------------------------------------------------------------------------------------------------------
// The pacing rule
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn presence_and_control_go_first_and_only_the_newest_presence_is_kept() {
    let mut p = Pacer::new(MS20);
    p.bulk(&[7; 100_000]);
    p.apply(&[8; 10]);
    p.control(vec![1]);
    p.presence(5, vec![50]);
    p.presence(6, vec![60]);
    p.presence(5, vec![51]); // replaces the first, and keeps its place in the order
    assert_eq!(p.presence_waiting(), 2);
    // However much is unsent, presence and control still go.
    let full = usize::MAX;
    assert_eq!(p.next(full), Some((Class::Presence, vec![51])));
    assert_eq!(p.next(full), Some((Class::Presence, vec![60])));
    assert_eq!(p.next(full), Some((Class::Control, vec![1])));
    assert_eq!(p.next(full), None, "the streams wait for room");
    assert_eq!(p.next(FIRST_LIMIT), None);
    assert!(p.next(0).is_some());
}

#[test]
fn chunks_keep_the_unsent_bytes_under_the_limit_and_are_never_over_16_kib() {
    let mut rng = common::World::new(71, 1).rng;
    for rule in [
        MS20,
        Rule::Fixed(MAX_LIMIT),
        Rule::Fixed(1 << 20),
        Rule::Fixed(MIN_LIMIT),
    ] {
        let mut p = Pacer::new(rule);
        p.bulk(&vec![1; 2_000_000]);
        let mut unsent = 0usize;
        let mut taken = 0usize;
        for _ in 0..20_000 {
            if let Some((class, bytes)) = p.next(unsent) {
                assert_eq!(class, Class::Bulk);
                assert!(bytes.len() <= MAX_CHUNK);
                assert!(
                    unsent + bytes.len() < p.limit(),
                    "{unsent} + {} under {}",
                    bytes.len(),
                    p.limit()
                );
                unsent += bytes.len();
                taken += bytes.len();
            }
            // The connection sends a random amount.
            unsent -= (rng.below(3_000) as usize).min(unsent);
        }
        assert!(taken > 1_000_000, "{rule:?}: only {taken} bytes went out");
    }
}

#[test]
fn accepted_changes_go_before_bulk_and_bulk_is_never_starved() {
    let (a, b) = (Class::Apply, Class::Bulk);
    // Both long: eight chunks of changes, one of bulk, and again.
    let mut p = Pacer::new(Rule::Fixed(MAX_LIMIT));
    p.bulk(&vec![3; MAX_CHUNK * 40]);
    p.apply(&vec![2; MAX_CHUNK * 40]);
    let order: Vec<Class> = (0..27).map(|_| p.next(0).unwrap().0).collect();
    let mut want = Vec::new();
    for _ in 0..3 {
        want.extend([a; 8]);
        want.push(b);
    }
    assert_eq!(order, want);

    // A load is under way and a few changes arrive: they go at once, then the load goes on.
    let mut p = Pacer::new(Rule::Fixed(MAX_LIMIT));
    p.bulk(&vec![3; MAX_CHUNK * 40]);
    let before: Vec<Class> = (0..5).map(|_| p.next(0).unwrap().0).collect();
    assert_eq!(before, [b; 5]);
    p.apply(&vec![2; MAX_CHUNK * 3]);
    let after: Vec<Class> = (0..6).map(|_| p.next(0).unwrap().0).collect();
    assert_eq!(after, [a, a, a, b, b, b]);

    // If the stream whose turn it is has no chunk that fits, the other's whole remainder may go.
    let mut p = Pacer::new(Rule::Fixed(MIN_LIMIT));
    p.apply(&vec![2; 10_000]);
    p.bulk(&[3; 100]);
    // 3,900 of 4,096 are unsent: 195 bytes of room, too little for a chunk of changes, enough for all the bulk.
    assert_eq!(p.next(3_900), Some((b, vec![3; 100])));
    assert_eq!(p.next(3_900), None);
}

#[test]
fn the_limit_follows_what_the_connection_sends() {
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut p = Pacer::new(MS20);
    assert_eq!(p.limit(), FIRST_LIMIT, "before the first estimate");
    // 5 Mbit/s is 625 bytes a millisecond: 20 ms of that is 12,500 bytes.
    for ms in 0..=200 {
        p.sent(at(ms), 625, true);
    }
    assert!((12_000..13_100).contains(&p.limit()), "limit {}", p.limit());
    // An idle stretch says nothing and changes nothing.
    for ms in 201..=2_000 {
        p.sent(at(ms), 0, false);
    }
    assert!((12_000..13_100).contains(&p.limit()), "limit {}", p.limit());
    // The connection becomes four times slower. The newest window counts for a quarter, so the limit comes
    // down over several windows, and ends near 20 ms of the new speed.
    let mut limits = Vec::new();
    for ms in 2_001..=3_000 {
        p.sent(at(ms), 156, true);
        limits.push(p.limit());
    }
    assert!(limits.windows(2).all(|w| w[1] <= w[0]));
    assert!(limits[200] > limits[999]);
    assert_eq!(
        p.limit(),
        MIN_LIMIT,
        "20 ms of 1.25 Mbit/s is under the floor"
    );
    // A very fast connection is held at the ceiling, a fixed rule ignores the speed.
    for ms in 3_001..=4_000 {
        p.sent(at(ms), 100_000, true);
    }
    assert_eq!(p.limit(), MAX_LIMIT);
    let mut fixed = Pacer::new(Rule::Fixed(1 << 20));
    fixed.sent(at(0), 1, true);
    fixed.sent(at(100), 1, true);
    assert_eq!(fixed.limit(), 1 << 20);
}

// ---------------------------------------------------------------------------------------------------------------
// Cursor delay in the model
// ---------------------------------------------------------------------------------------------------------------

fn model(rule: Rule, first_limit: usize, queues: Queues, mbit: f64) -> link_model::Report {
    let seconds = 20.0;
    let bulk = (mbit * 1e6 / 8.0 * seconds * 1.5) as usize;
    link_model::run(
        rule,
        first_limit,
        queues,
        mbit * 1e6,
        bulk,
        Duration::from_secs_f64(seconds),
    )
}

/// The 95th percentile after the first second, in milliseconds.
fn steady_p95(r: &link_model::Report) -> f64 {
    let steady = link_model::Report {
        delays: r.delays[30..].to_vec(),
        ..r.clone()
    };
    steady.delay_ms().1
}

#[test]
fn in_the_model_the_20_ms_rule_keeps_cursors_prompt_during_a_load() {
    // The design's table (section 04) for two queues, in ms: 0.25 Mbit/s 262, 0.5 131, 1 66, 2 40, 5 40,
    // 10 33, 20 27, 50 13, 100 6.6. The model gives the same within a few milliseconds.
    for (mbit, design) in [
        (0.25, 262.0),
        (0.5, 131.0),
        (1.0, 66.0),
        (2.0, 40.0),
        (5.0, 40.0),
        (10.0, 33.0),
        (20.0, 27.0),
        (50.0, 13.0),
        (100.0, 6.6),
    ] {
        let r = model(MS20, FIRST_LIMIT, Queues::Two, mbit);
        let p95 = steady_p95(&r);
        assert!(
            (p95 - design).abs() <= 6.0,
            "{mbit} Mbit/s: {p95:.0} ms, the design says {design}"
        );
        assert_eq!(r.replaced, 0, "no cursor message was replaced");
        // Counting every unsent byte in one place is never worse.
        let one = steady_p95(&model(MS20, FIRST_LIMIT, Queues::One, mbit));
        assert!(
            one <= p95 + 1.0,
            "{mbit} Mbit/s: one queue {one:.0} ms, two {p95:.0} ms"
        );
    }
}

#[test]
fn in_the_model_no_pacing_makes_cursors_wait_for_the_whole_buffer() {
    for mbit in [1.0, 5.0, 20.0] {
        let paced = steady_p95(&model(MS20, FIRST_LIMIT, Queues::Two, mbit));
        let megabyte = steady_p95(&model(Rule::Fixed(1 << 20), FIRST_LIMIT, Queues::One, mbit));
        // A full megabyte ahead of every cursor message: 8.4 s at 1 Mbit/s, 0.42 s at 20.
        let want = (1 << 20) as f64 * 8.0 / (mbit * 1e6) * 1e3;
        assert!(
            (megabyte - want).abs() < want * 0.02,
            "{mbit} Mbit/s: {megabyte:.0} ms"
        );
        assert!(megabyte > 10.0 * paced);
    }
}

#[test]
fn in_the_model_the_start_of_a_load_is_the_slow_part_on_a_slow_link() {
    // Before the first estimate the limit is 16 KiB (the design's value). On a slow link that is far more than
    // 20 ms of sending, and the first cursor messages wait behind it.
    for (mbit, over) in [(0.25, 900.0), (0.5, 450.0), (1.0, 200.0)] {
        let design = model(MS20, FIRST_LIMIT, Queues::Two, mbit);
        let small = model(MS20, MIN_LIMIT, Queues::Two, mbit);
        let (worst, worst_small) = (design.delay_ms().2, small.delay_ms().2);
        assert!(
            worst > over,
            "{mbit} Mbit/s: the worst delay is {worst:.0} ms"
        );
        // Starting at the floor instead, the start is no worse than the steady state.
        assert!(
            worst_small <= steady_p95(&small) + 10.0,
            "{mbit} Mbit/s: {worst_small:.0} ms"
        );
        // Either way the limit settles within half a second, as the design asks.
        assert!(design.settled_after.unwrap() <= Duration::from_millis(500));
    }
    // On a fast link the starting limit does not matter.
    let fast = model(MS20, FIRST_LIMIT, Queues::Two, 20.0);
    assert!(fast.delay_ms().2 < 35.0);
}
