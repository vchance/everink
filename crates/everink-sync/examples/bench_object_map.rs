//! Timing and size run for spike question 7: does Everink's own object map hold up at 100,000 strokes?
//!
//! Run with `cargo run --release --example bench_object_map [objects] [depth]`. The defaults are 100,000
//! objects with anchors at depth 0; `100000 100` puts the anchors 100 levels down.
//! Every timed step is repeated and gives its spread: opening a state nine times, each pass over all the
//! objects five times.

use std::hint::black_box;
use std::time::Instant;

use everink_core::{Axis, DepthCap, Level, Position};
use everink_sync::object::IDENTITY_LINEAR;
use everink_sync::rng::SplitMix64;
use everink_sync::{
    Applied, At, Body, Change, ClientChangeId, ClientId, NewObject, Object, ObjectId, Op, Outcome,
    PackedPoints, Paint, Point, Replica, Room, Seq, StrokeBrush, UserId, Value, Xform,
};

const POINTS_PER_STROKE: usize = 200;
const OPEN_RUNS: usize = 9;
/// How often each pass over all the objects is repeated.
const PASS_RUNS: usize = 5;

/// Resident memory of this process in bytes, as the operating system reports it, or `None` where `ps` is not
/// available. Used as a second opinion on the room's own estimate.
fn resident_bytes() -> Option<usize> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib: usize = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    Some(kib * 1024)
}

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        v[0],
        v[v.len() / 2],
        v[(v.len() * 95 / 100).min(v.len() - 1)],
    )
}

fn anchor(rng: &mut SplitMix64, depth: i32) -> Position {
    let near = Position {
        level: Level(0),
        x: Axis::from_i128((rng.next_u64() >> 8) as i128),
        y: Axis::from_i128((rng.next_u64() >> 8) as i128),
    };
    let mut p = near.at_level(Level(depth));
    p.x = p.x.add(&Axis::from_i128(rng.next_u64() as i128));
    p.y = p.y.add(&Axis::from_i128(rng.next_u64() as i128));
    p
}

/// A pen stroke: a random walk whose steps are a few hundred lattice units, about what a hand moves between
/// two samples at a normal zoom.
fn stroke(rng: &mut SplitMix64) -> PackedPoints {
    let (mut x, mut y) = (0i32, 0i32);
    let points: Vec<Point> = (0..POINTS_PER_STROKE)
        .map(|_| {
            x += rng.below(801) as i32 - 400;
            y += rng.below(801) as i32 - 400;
            Point {
                x,
                y,
                pressure: 1 + rng.below(255) as u8,
            }
        })
        .collect();
    PackedPoints::pack(&points).expect("under the point limit")
}

fn object_id(rng: &mut SplitMix64) -> ObjectId {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&rng.next_u64().to_le_bytes());
    b[8..].copy_from_slice(&rng.next_u64().to_le_bytes());
    ObjectId(b)
}

/// The change that draws stroke number `i`, on top of everything.
fn new_stroke(rng: &mut SplitMix64, i: usize, depth: i32) -> Change {
    Change {
        id: ClientChangeId(i as u64 + 1),
        observed: Seq(0),
        ops: vec![Op::Create(NewObject {
            id: object_id(rng),
            parent: None,
            at: At::Front,
            xform: Xform {
                anchor: anchor(rng, depth),
                linear: IDENTITY_LINEAR,
            },
            name: None,
            body: Body::Stroke {
                brush: StrokeBrush {
                    brush: 1,
                    base_width: 3 << 16,
                    style_ver: 1,
                },
                points: stroke(rng),
                paint: Paint { rgba: 0x2020_20ff },
            },
        })],
    }
}

fn accept(room: &mut Room, client: ClientId, change: &Change) -> Applied {
    match room.submit(client, UserId(1), change) {
        Outcome::Accepted(a) => a,
        other => panic!("{other:?}"),
    }
}

fn per_object_us(t: Instant, n: usize) -> f64 {
    t.elapsed().as_secs_f64() * 1e6 / n as f64
}

fn run(n: usize, depth: i32) {
    let cap = DepthCap::DEFAULT;
    let client = ClientId([1; 16]);
    let seed = 42 + depth as u64;
    println!("\n== {n} strokes of {POINTS_PER_STROKE} points, anchors {depth} levels down ==");

    // 0. Memory, measured first, while the process holds nothing else. Each change is made, given to the room
    //    and dropped, so the room is the only owner of what it keeps.
    {
        let mut rng = SplitMix64(seed);
        let before = resident_bytes();
        let mut room = Room::new(cap, 7);
        for i in 0..n {
            let change = new_stroke(&mut rng, i, depth);
            black_box(accept(&mut room, client, &change));
        }
        let after = resident_bytes();
        let (objects, index) = room.memory_estimate();
        println!(
            "size of the object struct:          {:8} bytes",
            std::mem::size_of::<Object>()
        );
        println!(
            "memory per object, estimated:       {:8.0} bytes ({:.0} in the map and the object, {:.0} in the children index)",
            (objects + index) as f64 / n as f64,
            objects as f64 / n as f64,
            index as f64 / n as f64
        );
        match (before, after) {
            (Some(b), Some(a)) => println!(
                "memory per object, resident:        {:8.0} bytes (growth of the process while the room was filled)",
                a.saturating_sub(b) as f64 / n as f64
            ),
            _ => println!("memory per object, resident:        not available (no `ps`)"),
        }
    }

    // The timed passes. The changes are built first, so that building them is not in the timings.
    let mut rng = SplitMix64(seed);
    let changes: Vec<Change> = (0..n).map(|i| new_stroke(&mut rng, i, depth)).collect();
    let ids: Vec<ObjectId> = changes
        .iter()
        .map(|c| match &c.ops[0] {
            Op::Create(new) => new.id,
            _ => unreachable!(),
        })
        .collect();

    // 1. The server accepts every stroke. Each run fills a new room.
    let mut times = Vec::new();
    let mut filled = None;
    for _ in 0..PASS_RUNS {
        let mut room = Room::new(cap, 7);
        let mut creates: Vec<Applied> = Vec::with_capacity(n);
        let t = Instant::now();
        for c in &changes {
            creates.push(accept(&mut room, client, c));
        }
        times.push(per_object_us(t, n));
        filled = Some((room, creates));
    }
    let (mut room, creates) = filled.expect("at least one run");
    line("server accepts a new stroke:", times);
    let packed: usize = room
        .objects()
        .map(|o| match o.body() {
            Body::Stroke { points, .. } => points.bytes().len(),
            Body::Group { .. } => 0,
        })
        .sum();
    println!(
        "packed points per stroke:           {:8.0} bytes ({:.2} per point)",
        packed as f64 / n as f64,
        packed as f64 / (n * POINTS_PER_STROKE) as f64
    );
    let longest_key = room
        .objects()
        .map(|o| o.slot().order.as_bytes().len())
        .max()
        .unwrap_or(0);
    println!("longest order key:                  {longest_key:8} bytes");

    // 2. The canonical bytes of the whole state, and opening a room from them.
    let t = Instant::now();
    let bytes = room.encode_all();
    let encode_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "state as bytes:                     {:8.0} bytes per object ({:.1} MB in all; {:.0} per object without the points; written in {encode_ms:.1} ms, one run)",
        bytes.len() as f64 / n as f64,
        bytes.len() as f64 / 1e6,
        (bytes.len() - packed) as f64 / n as f64
    );
    let mut open = Vec::new();
    for _ in 0..OPEN_RUNS {
        let t = Instant::now();
        let opened = Room::decode_all(black_box(&bytes), cap, 7).expect("its own bytes");
        open.push(t.elapsed().as_secs_f64() * 1e3);
        assert_eq!(opened.len(), n);
        black_box(opened);
    }
    let (min, med, p95) = stats(open);
    println!(
        "open from bytes (decode, check, index): {med:6.1} ms median of {OPEN_RUNS} (min {min:.1}, p95 {p95:.1}; {:.2} us per object)",
        med * 1e3 / n as f64
    );

    // 3. A client's copy merges every stroke. Each run fills a new copy.
    let mut times = Vec::new();
    let mut replica = Replica::new(ClientId([2; 16]), UserId(2));
    for _ in 0..PASS_RUNS {
        replica = Replica::new(ClientId([2; 16]), UserId(2));
        let t = Instant::now();
        for a in &creates {
            replica.apply(a);
        }
        times.push(per_object_us(t, n));
    }
    line("client copy merges a new stroke:", times);
    assert_eq!(replica.encode_objects(), room.encode_objects());
    drop(creates);

    // 4. One register write per object: a new transform, which is what dragging an object on the surface
    //    sends. (Moving it in the tree, `SetSlot`, is not timed.) Each run writes every object once more.
    let (mut server, mut merge, mut stale) = (Vec::new(), Vec::new(), Vec::new());
    for run in 0..PASS_RUNS {
        let writes: Vec<Change> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Change {
                id: ClientChangeId((n * (run + 1) + i) as u64 + 1),
                observed: room.head(),
                ops: vec![Op::Set {
                    id: *id,
                    value: Value::Xform(Xform {
                        anchor: anchor(&mut rng, depth),
                        linear: IDENTITY_LINEAR,
                    }),
                    if_stamp: None,
                }],
            })
            .collect();
        let mut log: Vec<Applied> = Vec::with_capacity(n);
        let t = Instant::now();
        for c in &writes {
            log.push(accept(&mut room, client, c));
        }
        server.push(per_object_us(t, n));
        let t = Instant::now();
        for a in &log {
            replica.apply(a);
        }
        merge.push(per_object_us(t, n));
        assert_eq!(replica.encode_objects(), room.encode_objects());

        // 5. The same writes again, newest first: each must be looked at and ignored.
        let t = Instant::now();
        for a in log.iter().rev() {
            replica.apply(a);
        }
        stale.push(per_object_us(t, n));
        assert_eq!(replica.encode_objects(), room.encode_objects());
    }
    line("server accepts a transform write:", server);
    line("client copy merges a transform write:", merge);
    line("client copy ignores a stale write:", stale);
    room.check_tree().expect("a tree");
}

/// Prints the minimum, median and maximum of the runs of one pass, in microseconds per object.
fn line(what: &str, times: Vec<f64>) {
    let runs = times.len();
    let (min, med, _) = stats(times.clone());
    let max = times.iter().cloned().fold(0.0, f64::max);
    println!("{what:38} {med:6.2} us each, median of {runs} runs (min {min:.2}, max {max:.2})");
}

fn main() {
    let arg = |i: usize| std::env::args().nth(i).and_then(|s| s.parse::<i64>().ok());
    let n = arg(1).unwrap_or(100_000) as usize;
    let depth = arg(2).unwrap_or(0) as i32;
    println!(
        "everink-sync object map, {} {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    // One depth per process, so that the memory figure starts from a process that holds nothing.
    run(n, depth);
}
