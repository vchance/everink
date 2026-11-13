//! Spike question 9: how fast is storage?
//!
//! `cargo run --release --locked --manifest-path spike/store-trial/Cargo.toml [sizes] [directory]`
//! (default sizes `10000,100000,1000000`; default directory `spike/store-trial/target/data`).
//!
//! Part 1 times a commit of the log, for each way of syncing and for batches of 1, 10 and 100 changes. Part 2,
//! for each size, fills a room with that many strokes, writes a whole snapshot, 600 more strokes and a snapshot
//! of the chunks they dirtied, and a log tail of 1,000 changes; then opens the room from the file several times
//! and says where the time went. The room opened must be the room written, byte for byte, or the run fails.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use everink_core::DepthCap;
use everink_sync::rng::SplitMix64;
use everink_sync::{Applied, At, Change, ClientId, Outcome, Room, UserId};
use store_trial::{LoadStats, LogRow, Store, Sync, dirty_chunks, make};

const CAP: DepthCap = DepthCap::DEFAULT;
const CLIENT: ClientId = ClientId([1; 16]);
const USER: UserId = UserId(1);
const SEED: u64 = 7;

/// A room and the file it is written to.
struct Writer {
    room: Room,
    rng: SplitMix64,
    next: u64,
    dirty: BTreeSet<u32>,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            room: Room::new(CAP, SEED),
            rng: SplitMix64(42),
            next: 1,
            dirty: BTreeSet::new(),
        }
    }

    /// Draws one stroke on top and returns the change and what the room made of it.
    fn stroke(&mut self) -> (Change, Applied) {
        let change = make::stroke(&mut self.rng, self.next, At::Front);
        self.next += 1;
        let applied = match self.room.submit(CLIENT, USER, &change) {
            Outcome::Accepted(a) => a,
            other => panic!("{other:?}"),
        };
        dirty_chunks(&self.room, &applied, &mut self.dirty);
        (change, applied)
    }

    /// Draws `count` strokes and appends them to the log, `batch` to a transaction. Returns the milliseconds
    /// each append took.
    fn log(&mut self, store: &mut Store, count: usize, batch: usize) -> Vec<f64> {
        let mut took = Vec::with_capacity(count / batch);
        for _ in 0..count / batch {
            let made: Vec<(Change, Applied)> = (0..batch).map(|_| self.stroke()).collect();
            let rows: Vec<LogRow<'_>> = made
                .iter()
                .map(|(change, applied)| LogRow {
                    seq: applied.seq,
                    time_ms: 0,
                    user: USER,
                    client: CLIENT,
                    change,
                })
                .collect();
            let t = Instant::now();
            store.append(&rows).expect("the log takes the rows");
            took.push(t.elapsed().as_secs_f64() * 1e3);
        }
        took
    }
}

/// Median, 95th and 99th percentile, and the most.
fn spread(v: &[f64]) -> (f64, f64, f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: usize| s[(s.len() * p / 100).min(s.len() - 1)];
    (at(50), at(95), at(99), s[s.len() - 1])
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn file_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn fresh(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{name}{suffix}")));
    }
    path
}

fn commits(dir: &Path) {
    println!("\n== Part 1: one commit of the log (a stroke of 200 points is one change) ==");
    for (name, sync, count) in [
        ("synchronous=FULL (the design's)", Sync::Full, 400),
        (
            "synchronous=FULL, fullfsync=ON",
            Sync::FullFsync,
            if cfg!(target_os = "macos") { 200 } else { 0 },
        ),
        (
            "synchronous=NORMAL (no sync; not durable)",
            Sync::Normal,
            2_000,
        ),
    ] {
        if count == 0 {
            println!("{name}: not run; it differs from FULL only on macOS");
            continue;
        }
        for batch in [1usize, 10, 100] {
            let path = fresh(dir, "commit.sqlite");
            let mut store = Store::open(&path, sync).expect("the file opens");
            let mut w = Writer::new();
            w.log(&mut store, batch * 20, batch); // not counted
            let took = w.log(&mut store, batch * count, batch);
            let (med, p95, p99, most) = spread(&took);
            println!(
                "{name}, {batch:3} changes a commit: median {med:.3} ms, 95th {p95:.3}, 99th {p99:.3}, most {most:.3}; {:.1} µs a change; {:.0} commits a second",
                med * 1e3 / batch as f64,
                1e3 / med
            );
        }
    }
}

fn room_of(dir: &Path, size: usize) {
    println!("\n== Part 2: a room of {size} strokes ==");
    let path = fresh(dir, &format!("room-{size}.sqlite"));
    let mut w = Writer::new();
    let t = Instant::now();
    for _ in 0..size {
        w.stroke();
    }
    println!(
        "filling the room in memory: {:.2} s (not storage)",
        t.elapsed().as_secs_f64()
    );

    // The whole snapshot.
    let mut store = Store::open(&path, Sync::Full).expect("the file opens");
    w.dirty.clear();
    let s = store.snapshot(&w.room, None, 0).expect("a snapshot");
    store.checkpoint().expect("a checkpoint");
    println!(
        "whole snapshot: {} chunks, {:.1} MB of objects stored as {:.1} MB ({:.0}% after LZ4; {:.0} bytes an object), manifest {:.1} KB; building the chunks {:.0} ms, the transaction {:.0} ms",
        s.chunks_written,
        s.raw_bytes as f64 / 1e6,
        s.stored_bytes as f64 / 1e6,
        s.stored_bytes as f64 * 100.0 / s.raw_bytes as f64,
        s.stored_bytes as f64 / size as f64,
        s.manifest_bytes as f64 / 1e3,
        s.build_ms,
        s.commit_ms
    );
    println!("the file: {:.1} MB", file_bytes(&path) as f64 / 1e6);

    // 600 strokes more, and a snapshot of what they dirtied (the design's example).
    w.log(&mut store, 600, 100);
    let dirty = std::mem::take(&mut w.dirty);
    let s = store
        .snapshot(&w.room, Some(&dirty), 0)
        .expect("a snapshot");
    println!(
        "snapshot after 600 more strokes: wrote {} of {} chunks ({:.0} KB) and a manifest of {:.1} KB; building {:.1} ms (of which sorting every object by ordinal), the transaction {:.2} ms",
        s.chunks_written,
        s.chunks_in_manifest,
        s.stored_bytes as f64 / 1e3,
        s.manifest_bytes as f64 / 1e3,
        s.build_ms,
        s.commit_ms
    );

    // A commit on a file of this size, and a log tail of 1,000 changes in all.
    let took = w.log(&mut store, 300, 1);
    let (med, p95, p99, most) = spread(&took);
    println!(
        "one change a commit on this file, synchronous=FULL: median {med:.3} ms, 95th {p95:.3}, 99th {p99:.3}, most {most:.3}"
    );
    w.log(&mut store, 700, 100);
    store.checkpoint().expect("a checkpoint");
    println!(
        "the file with a log tail of 1,000 changes: {:.1} MB",
        file_bytes(&path) as f64 / 1e6
    );
    drop(store);

    // Open it again, several times. The room written is dropped first: two of them at a million objects are
    // more memory than the measurement should need.
    let expect = blake3::hash(&w.room.encode_all());
    let head = w.room.head();
    drop(w);
    let runs = if size >= 1_000_000 { 3 } else { 5 };
    let mut all: Vec<(f64, LoadStats)> = Vec::new();
    for run in 0..runs {
        let t = Instant::now();
        let store = Store::open(&path, Sync::Full).expect("the file opens");
        let (room, stats) = store.load(CAP, SEED).expect("the room opens");
        let total = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(room.head(), head);
        if run == 0 {
            assert!(
                blake3::hash(&room.encode_all()) == expect,
                "the room opened is not the room written"
            );
        }
        all.push((total, stats));
    }
    let part = |f: fn(&LoadStats) -> f64| median(all.iter().map(|(_, s)| f(s)).collect());
    let stats = all[0].1;
    println!(
        "opening it ({} chunks, {} objects, {} log rows; median of {runs}; the file is in the operating system's cache): {:.0} ms",
        stats.chunks,
        stats.objects,
        stats.log_rows,
        median(all.iter().map(|(t, _)| *t).collect())
    );
    println!(
        "  of which reading the chunks from SQLite {:.0} ms, checking hashes and decompressing {:.0} ms, decoding the objects {:.0} ms, building the room {:.0} ms, replaying the log {:.1} ms ({:.1} µs a change)",
        part(|s| s.read_ms),
        part(|s| s.verify_ms),
        part(|s| s.decode_ms),
        part(|s| s.build_ms),
        part(|s| s.replay_ms),
        part(|s| s.replay_ms) * 1e3 / stats.log_rows as f64
    );
    println!("check: the room opened is the room written, byte for byte");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sizes: Vec<usize> = args
        .next()
        .unwrap_or_else(|| "10000,100000,1000000".to_string())
        .split(',')
        .map(|s| s.parse().expect("sizes are numbers with commas between"))
        .collect();
    let dir = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "spike/store-trial/target/data".to_string()),
    );
    std::fs::create_dir_all(&dir).expect("the directory can be made");
    println!(
        "store trial, {} {}; SQLite {}; files in {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        rusqlite::version(),
        dir.display()
    );
    commits(&dir);
    for size in sizes {
        room_of(&dir, size);
    }
    for entry in std::fs::read_dir(&dir)
        .expect("the directory is there")
        .flatten()
    {
        let _ = std::fs::remove_file(entry.path());
    }
}
