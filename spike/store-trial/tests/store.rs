//! What the store must do whatever it costs: give the room back exactly, and refuse a damaged file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use everink_core::DepthCap;
use everink_sync::rng::SplitMix64;
use everink_sync::{
    Applied, At, Change, ClientChangeId, ClientId, Op, Outcome, Room, UserId, Value, Xform,
};
use store_trial::{LogRow, Store, StoreError, Sync, dirty_chunks, make};

const CAP: DepthCap = DepthCap::DEFAULT;
const CLIENT: ClientId = ClientId([1; 16]);
const USER: UserId = UserId(7);

/// A file of its own for each test, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("store-trial-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("room.sqlite")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A room being written to: every accepted change goes to the log.
struct Writer {
    room: Room,
    store: Store,
    rng: SplitMix64,
    next: u64,
    dirty: BTreeSet<u32>,
}

impl Writer {
    fn new(scratch: &Scratch) -> Writer {
        Writer {
            room: Room::new(CAP, 99),
            store: Store::open(&scratch.file(), Sync::Normal).unwrap(),
            rng: SplitMix64(3),
            next: 1,
            dirty: BTreeSet::new(),
        }
    }

    fn accept(&mut self, change: &Change) -> Applied {
        let applied = match self.room.submit(CLIENT, USER, change) {
            Outcome::Accepted(a) => a,
            other => panic!("{other:?}"),
        };
        self.store
            .append(&[LogRow {
                seq: applied.seq,
                time_ms: 0,
                user: USER,
                client: CLIENT,
                change,
            }])
            .unwrap();
        dirty_chunks(&self.room, &applied, &mut self.dirty);
        applied
    }

    /// Draws strokes: on top, at the bottom, and between two others, so that every way of making an order
    /// key is used; and moves an older stroke now and then.
    fn draw(&mut self, strokes: usize) {
        for i in 0..strokes {
            let at = match i % 3 {
                0 => At::Front,
                1 => At::Back,
                _ => At::Between(None, None),
            };
            let change = make::stroke(&mut self.rng, self.next, at);
            self.next += 1;
            self.accept(&change);
            if i % 7 == 6 {
                let ids: Vec<_> = self.room.objects().map(|o| o.id()).collect();
                let id = *ids.iter().min().unwrap();
                let change = Change {
                    id: ClientChangeId(self.next),
                    observed: self.room.head(),
                    ops: vec![Op::Set {
                        id,
                        value: Value::Xform(Xform {
                            anchor: make::anchor(&mut self.rng),
                            linear: everink_sync::object::IDENTITY_LINEAR,
                        }),
                        if_stamp: None,
                    }],
                };
                self.next += 1;
                self.accept(&change);
            }
        }
    }

    fn snapshot(&mut self, whole: bool) -> store_trial::SnapshotStats {
        let dirty = std::mem::take(&mut self.dirty);
        self.store
            .snapshot(&self.room, if whole { None } else { Some(&dirty) }, 0)
            .unwrap()
    }
}

fn reopen(scratch: &Scratch) -> Result<Room, StoreError> {
    Store::open(&scratch.file(), Sync::Normal)?
        .load(CAP, 99)
        .map(|(room, _)| room)
}

#[test]
fn a_room_comes_back_exactly_from_the_log_alone() {
    let scratch = Scratch::new("log");
    let mut w = Writer::new(&scratch);
    w.draw(700);
    let opened = reopen(&scratch).unwrap();
    assert_eq!(opened.head(), w.room.head());
    assert!(opened.encode_all() == w.room.encode_all());
}

#[test]
fn a_room_comes_back_exactly_from_a_snapshot_and_the_log_after_it() {
    let scratch = Scratch::new("snap");
    let mut w = Writer::new(&scratch);
    w.draw(900);
    let first = w.snapshot(true);
    assert_eq!(first.chunks_written, first.chunks_in_manifest);
    assert!(first.chunks_in_manifest >= 3, "{first:?}");
    // From the snapshot alone.
    assert!(reopen(&scratch).unwrap().encode_all() == w.room.encode_all());
    // And with changes after it, which the log holds and the snapshot does not.
    w.draw(400);
    let (opened, stats) = Store::open(&scratch.file(), Sync::Normal)
        .unwrap()
        .load(CAP, 99)
        .unwrap();
    assert!(stats.log_rows >= 400, "{stats:?}");
    assert_eq!(stats.objects, 900);
    assert!(opened.encode_all() == w.room.encode_all());
    opened.check_tree().unwrap();
}

#[test]
fn a_second_snapshot_writes_only_the_chunks_that_changed_and_the_log_can_be_trimmed() {
    let scratch = Scratch::new("dirty");
    let mut w = Writer::new(&scratch);
    w.draw(2_000);
    let first = w.snapshot(true);
    // 300 new strokes fill at most two chunks; the moves that go with them touch one old chunk.
    w.draw(300);
    let second = w.snapshot(false);
    assert!(second.chunks_in_manifest > first.chunks_in_manifest);
    assert!(
        second.chunks_written <= 4,
        "wrote {} of {} chunks",
        second.chunks_written,
        second.chunks_in_manifest
    );
    assert!(reopen(&scratch).unwrap().encode_all() == w.room.encode_all());
    // A snapshot made the short way holds the same as one made of every chunk.
    let whole = w.snapshot(true);
    assert_eq!(whole.chunks_in_manifest, second.chunks_in_manifest);
    assert!(reopen(&scratch).unwrap().encode_all() == w.room.encode_all());
    // Trimming removes the rows up to the second-newest snapshot and nothing that is still needed.
    w.draw(50);
    let removed = w.store.trim_log().unwrap();
    assert!(removed >= 2_300, "removed {removed}");
    assert!(reopen(&scratch).unwrap().encode_all() == w.room.encode_all());
}

#[test]
fn replay_needs_the_rooms_random_state() {
    // The room draws random numbers when it makes an order key between two others. A room opened from a
    // snapshot must go on from the same state, or the changes after the snapshot give other keys than they did
    // the first time. The store saves the state in the manifest; this shows what happens without it.
    let mut rng = SplitMix64(11);
    let mut room = Room::new(CAP, 99);
    let mut number = 1;
    let mut draw = |room: &mut Room, at: At| {
        let change = make::stroke(&mut rng, number, at);
        number += 1;
        assert!(matches!(
            room.submit(CLIENT, USER, &change),
            Outcome::Accepted(_)
        ));
        change
    };
    for _ in 0..20 {
        draw(&mut room, At::Between(None, None));
    }
    let (head, next) = (room.head(), room.next_ordinal());
    let saved: Vec<_> = room.objects().cloned().collect();
    let state = room.rng_state();
    let later: Vec<Change> = (0..20)
        .map(|_| draw(&mut room, At::Between(None, None)))
        .collect();

    let replay = |seed: u64| {
        let mut opened = Room::from_objects(head, next, saved.clone(), CAP, seed).unwrap();
        for change in &later {
            assert!(matches!(
                opened.submit(CLIENT, USER, change),
                Outcome::Accepted(_)
            ));
        }
        opened.encode_all()
    };
    assert!(
        replay(state) == room.encode_all(),
        "with the saved state the replay is exact"
    );
    assert!(
        replay(99) != room.encode_all(),
        "with the first seed again the replay gives other order keys"
    );
}

#[test]
fn a_damaged_file_is_refused() {
    let build = |name: &str| {
        let scratch = Scratch::new(name);
        let mut w = Writer::new(&scratch);
        w.draw(600);
        w.snapshot(true);
        w.draw(100);
        scratch
    };
    let refused = |scratch: &Scratch, sql: &str, expect: &str| {
        let store = Store::open(&scratch.file(), Sync::Normal).unwrap();
        assert!(
            store.execute_for_test(sql).unwrap() >= 1,
            "{sql} changed nothing"
        );
        match store.load(CAP, 99) {
            Err(StoreError::Damaged(what)) => {
                assert!(what.contains(expect), "{what:?} does not say {expect:?}")
            }
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("a file damaged by {sql:?} was opened"),
        }
    };
    // One byte of a chunk changed.
    refused(
        &build("chunk"),
        "UPDATE chunk SET data = CAST(substr(data, 1, 40) || x'00ff00' || substr(data, 44) AS BLOB)
         WHERE hash = (SELECT hash FROM chunk LIMIT 1)",
        "does not match its hash",
    );
    // A chunk gone.
    refused(
        &build("gone"),
        "DELETE FROM chunk WHERE hash = (SELECT hash FROM chunk LIMIT 1)",
        "is missing",
    );
    // The manifest changed.
    refused(
        &build("manifest"),
        "UPDATE snap SET manifest = CAST(substr(manifest, 1, 30) || x'5a' || substr(manifest, 32) AS BLOB)",
        "checksum",
    );
    // A log row after the snapshot gone, and one changed.
    refused(
        &build("row"),
        "DELETE FROM log WHERE seq = (SELECT max(seq) - 5 FROM log)",
        "follows",
    );
    refused(
        &build("blob"),
        "UPDATE log SET blob = CAST(substr(blob, 1, 20) || x'5a' || substr(blob, 22) AS BLOB)
         WHERE seq = (SELECT max(seq) - 5 FROM log)",
        "checksum",
    );
    // A row before the snapshot does not matter: it is not read.
    let scratch = build("old");
    let store = Store::open(&scratch.file(), Sync::Normal).unwrap();
    store
        .execute_for_test("DELETE FROM log WHERE seq = 5")
        .unwrap();
    assert!(store.load(CAP, 99).is_ok());
}
