//! Cursor delay during a bulk load over real sockets (spike question 8).
//!
//! `cargo run --release --example paced_link [seconds per run] [runs]` (defaults 6 and 1).
//!
//! Three parts, all in this process, joined by TCP on the loopback interface:
//!
//! ```text
//! server writer --TCP--> relay (lets through so many bits a second) --TCP--> client
//! ```
//!
//! - The **server writer** has a room of strokes to send to a client that has just joined (the bulk load), and
//!   another person's cursor to pass on 30 times a second. A `Pacer` decides what is written next.
//! - The **relay** stands for a slow link: it reads from the server at a fixed rate and passes the bytes on.
//! - The **client** reads frames, puts the stroke stream back together and decodes every object into a copy, and
//!   records how long each cursor message took from the moment it was made.
//!
//! What this is and is not. The sockets, the frames and the objects are real. The link is not: there is no
//! loss and no congestion control worth the name on loopback. The design bounds the unsent bytes inside the
//! operating system with `TCP_NOTSENT_LOWAT`; the safe wrapper for it exists on Linux only, so here the socket
//! buffers are made as small as the system allows and the writer keeps the unsent bytes in a queue of its own,
//! handing a chunk to it whenever it is under the limit (the "two queues" of `link_model`). The buffer sizes the
//! system actually gave are printed, because whatever sits in them is delay the pacer cannot see.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use everink_core::{Axis, DepthCap, Level, Position};
use everink_sync::link_model::{CURSOR_BYTES, CURSOR_EVERY, FRAME_HEADER};
use everink_sync::object::IDENTITY_LINEAR;
use everink_sync::pacing::{Class, Pacer, Rule};
use everink_sync::rng::SplitMix64;
use everink_sync::{
    At, Body, Change, ClientChangeId, ClientId, NewObject, Object, ObjectId, Op, Outcome,
    PackedPoints, Paint, Point, Replica, Room, Seq, StrokeBrush, UserId, Xform,
};
use socket2::{Domain, Socket, Type};

const SMALL_BUFFER: usize = 4 * 1024;
const STROKES: usize = 30_000;
/// The relay reads this many bytes at a time: 8 ms of a 1 Mbit/s link.
const READ: usize = 1024;
/// Bytes the relay has read from the server, to see how much is waiting inside the operating system.
static RELAY_READ: AtomicUsize = AtomicUsize::new(0);

/// A room of strokes, and its objects as one stream of bytes: for each object a 4-byte length, then its
/// canonical bytes.
fn bulk_stream() -> (Room, Vec<u8>) {
    let mut rng = SplitMix64(8);
    let mut room = Room::new(DepthCap::DEFAULT, 8);
    for i in 0..STROKES {
        let (mut x, mut y) = (0i32, 0i32);
        let points: Vec<Point> = (0..200)
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
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&rng.next_u64().to_le_bytes());
        id[8..].copy_from_slice(&rng.next_u64().to_le_bytes());
        let change = Change {
            id: ClientChangeId(i as u64 + 1),
            observed: Seq(0),
            ops: vec![Op::Create(NewObject {
                id: ObjectId(id),
                parent: None,
                at: At::Front,
                xform: Xform {
                    anchor: Position {
                        level: Level(0),
                        x: Axis::from_i128((rng.next_u64() >> 8) as i128),
                        y: Axis::from_i128((rng.next_u64() >> 8) as i128),
                    },
                    linear: IDENTITY_LINEAR,
                },
                name: None,
                body: Body::Stroke {
                    brush: StrokeBrush {
                        brush: 1,
                        base_width: 3 << 16,
                        style_ver: 1,
                    },
                    points: PackedPoints::pack(&points).expect("under the point limit"),
                    paint: Paint { rgba: 0x2020_20ff },
                },
            })],
        };
        let outcome = room.submit(ClientId([1; 16]), UserId(1), &change);
        assert!(matches!(outcome, Outcome::Accepted(_)));
    }
    let mut stream = Vec::new();
    let mut objects: Vec<&Object> = room.objects().collect();
    objects.sort_by_key(|o| o.ordinal());
    for o in objects {
        let bytes = o.encode();
        stream.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        stream.extend_from_slice(&bytes);
    }
    (room, stream)
}

fn frame(class: Class, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(FRAME_HEADER + payload.len());
    f.push(match class {
        Class::Presence => 0,
        Class::Control => 1,
        Class::Apply => 2,
        Class::Bulk => 3,
    });
    f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    f.extend_from_slice(payload);
    f
}

/// The slow link. Reads from the server at `bits_per_second` and passes the bytes on to the client.
fn relay(
    server: SocketAddr,
    listen: TcpListener,
    bits_per_second: f64,
    ready: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> usize {
    let (mut to_client, _) = listen.accept().expect("the client connects");
    to_client.set_nodelay(true).ok();
    // A small receive buffer, set before connecting, so that little waits on this side of the server.
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("a socket");
    socket.set_recv_buffer_size(SMALL_BUFFER).ok();
    socket
        .connect(&server.into())
        .expect("the server is listening");
    // macOS sizes the buffer again when a loopback connection is made, so it is set once more.
    socket.set_recv_buffer_size(SMALL_BUFFER).ok();
    let given = socket.recv_buffer_size().unwrap_or(0);
    // Only now may the server start: anything it wrote before the buffer was made small would be taken whole.
    ready.store(true, Ordering::Relaxed);
    let mut from_server: TcpStream = socket.into();
    from_server
        .set_read_timeout(Some(Duration::from_millis(20)))
        .ok();

    let bytes_per_second = bits_per_second / 8.0;
    let mut allowance = 0.0f64;
    let mut last = Instant::now();
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        allowance += now.duration_since(last).as_secs_f64() * bytes_per_second;
        last = now;
        // Never more saved up than 5 ms of sending (or one read): a link does not send faster after a pause.
        allowance = allowance.min((bytes_per_second * 0.005).max(READ as f64));
        // Always a whole read of the same size. Reads of a few hundred bytes make macOS grow the loopback
        // buffer to hundreds of kilobytes, whatever size was asked for.
        if allowance < READ as f64 {
            thread::sleep(Duration::from_micros(300));
            continue;
        }
        let want = READ;
        match from_server.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => {
                allowance -= n as f64;
                RELAY_READ.fetch_add(n, Ordering::Relaxed);
                if to_client.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    given
}

struct ClientReport {
    /// Delay of each cursor message in milliseconds, with the time it arrived since the start.
    cursors: Vec<(f64, f64)>,
    objects: usize,
    bad_objects: usize,
}

fn client(relay: SocketAddr, start: Instant, stop: Arc<AtomicBool>) -> ClientReport {
    let mut stream = TcpStream::connect(relay).expect("the relay is listening");
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .ok();
    let cap = DepthCap::DEFAULT;
    let mut replica = Replica::new(ClientId([2; 16]), UserId(2));
    let mut report = ClientReport {
        cursors: Vec::new(),
        objects: 0,
        bad_objects: 0,
    };
    let mut wire: Vec<u8> = Vec::new();
    let mut bulk: Vec<u8> = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => wire.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(_) => break,
        }
        let now = start.elapsed().as_secs_f64() * 1e3;
        // Whole frames.
        let mut at = 0;
        while wire.len() - at >= FRAME_HEADER {
            let len =
                u32::from_le_bytes(wire[at + 1..at + 5].try_into().expect("four bytes")) as usize;
            if wire.len() - at < FRAME_HEADER + len {
                break;
            }
            let payload = &wire[at + FRAME_HEADER..at + FRAME_HEADER + len];
            match wire[at] {
                0 => {
                    let made = u64::from_le_bytes(payload[..8].try_into().expect("eight bytes"));
                    report.cursors.push((now - made as f64 / 1e3, now));
                }
                3 => bulk.extend_from_slice(payload),
                _ => {}
            }
            at += FRAME_HEADER + len;
        }
        wire.drain(..at);
        // Whole objects of the bulk stream.
        let mut at = 0;
        while bulk.len() - at >= 4 {
            let len = u32::from_le_bytes(bulk[at..at + 4].try_into().expect("four bytes")) as usize;
            if bulk.len() - at < 4 + len {
                break;
            }
            match Object::decode(&bulk[at + 4..at + 4 + len], &cap) {
                Ok(object) => {
                    replica.enter(&object);
                    report.objects += 1;
                }
                Err(_) => report.bad_objects += 1,
            }
            at += 4 + len;
        }
        bulk.drain(..at);
    }
    assert_eq!(replica.len(), report.objects);
    report
}

struct Run {
    cursors: Vec<(f64, f64)>,
    objects: usize,
    bad_objects: usize,
    limit: usize,
    send_buffer: usize,
    relay_buffer: usize,
    /// The most bytes that were ever written to the socket and not yet read by the relay: what waits inside the
    /// operating system, where the pacer cannot see it.
    in_system: usize,
    /// The pacer's last estimate of the link, in bits per second.
    estimate: f64,
}

fn run(rule: Rule, mbit: f64, seconds: f64, bulk: &[u8]) -> Run {
    let start = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let server = TcpListener::bind("127.0.0.1:0").expect("a port");
    let relay_listen = TcpListener::bind("127.0.0.1:0").expect("a port");
    let (server_addr, relay_addr) = (
        server.local_addr().expect("an address"),
        relay_listen.local_addr().expect("an address"),
    );
    let relay_thread = {
        let (ready, stop) = (ready.clone(), stop.clone());
        thread::spawn(move || relay(server_addr, relay_listen, mbit * 1e6, ready, stop))
    };
    let client_thread = {
        let stop = stop.clone();
        thread::spawn(move || client(relay_addr, start, stop))
    };

    // The server's side of the connection: as little as possible waits inside the operating system.
    let (stream, _) = server.accept().expect("the relay connects");
    let socket = Socket::from(stream);
    socket.set_tcp_nodelay(true).ok();
    socket.set_send_buffer_size(SMALL_BUFFER).ok();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    socket.set_tcp_notsent_lowat(SMALL_BUFFER as u32).ok();
    let send_buffer = socket.send_buffer_size().unwrap_or(0);
    socket.set_nonblocking(true).expect("a non-blocking socket");
    let mut stream: TcpStream = socket.into();

    while !ready.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_micros(100));
    }
    // The run is timed from here.
    let begin = start.elapsed();
    let mut pacer = Pacer::new(rule);
    pacer.bulk(bulk);
    // Bytes handed over by the pacer that the socket has not taken yet.
    let mut unsent: VecDeque<u8> = VecDeque::new();
    let mut next_cursor = Duration::ZERO;
    RELAY_READ.store(0, Ordering::Relaxed);
    let mut written = 0usize;
    let mut most_in_system = 0usize;
    next_cursor += begin;
    while (start.elapsed() - begin).as_secs_f64() < seconds {
        let now = start.elapsed();
        while next_cursor <= now {
            let mut message = vec![0u8; CURSOR_BYTES];
            message[..8].copy_from_slice(&(now.as_micros() as u64).to_le_bytes());
            pacer.presence(1, message);
            next_cursor += CURSOR_EVERY;
        }
        // The socket says only "room" or "no room": a chunk is handed over whenever the queue is under the limit.
        loop {
            let counted = if unsent.len() < pacer.limit() {
                0
            } else {
                usize::MAX
            };
            let Some((class, bytes)) = pacer.next(counted) else {
                break;
            };
            unsent.extend(frame(class, &bytes));
        }
        let (head, _) = unsent.as_slices();
        let wrote = match stream.write(head) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => 0,
            Err(_) => break,
        };
        unsent.drain(..wrote);
        written += wrote;
        most_in_system =
            most_in_system.max(written.saturating_sub(RELAY_READ.load(Ordering::Relaxed)));
        pacer.sent(Instant::now(), wrote, !unsent.is_empty());
        if wrote == 0 {
            thread::sleep(Duration::from_micros(200));
        }
    }
    stop.store(true, Ordering::Relaxed);
    let limit = pacer.limit();
    let estimate = pacer.rate().unwrap_or(0.0) * 8.0;
    drop(stream);
    let relay_buffer = relay_thread.join().expect("the relay ends");
    let report = client_thread.join().expect("the client ends");
    Run {
        cursors: report.cursors,
        objects: report.objects,
        bad_objects: report.bad_objects,
        limit,
        send_buffer,
        relay_buffer,
        in_system: most_in_system,
        estimate,
    }
}

/// Median, 95th percentile and maximum of the delays after the first second, and the worst in the first second.
fn summary(r: &Run) -> [f64; 4] {
    let mut steady: Vec<f64> = r
        .cursors
        .iter()
        .filter(|(_, at)| *at >= 1000.0)
        .map(|(d, _)| *d)
        .collect();
    steady.sort_by(|a, b| a.partial_cmp(b).expect("numbers"));
    let pick = |q: usize| {
        steady
            .get((steady.len() * q / 100).min(steady.len().saturating_sub(1)))
            .copied()
            .unwrap_or(f64::NAN)
    };
    let first = r
        .cursors
        .iter()
        .filter(|(_, at)| *at < 1000.0)
        .map(|(d, _)| *d)
        .fold(0.0, f64::max);
    [pick(50), pick(95), pick(100), first]
}

/// The middle, the least and the most of one figure over the repeated runs.
fn over_runs(values: impl Iterator<Item = f64>) -> (f64, f64, f64) {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (v[v.len() / 2], v[0], v[v.len() - 1])
}

fn main() {
    let arg = |i: usize| std::env::args().nth(i).and_then(|s| s.parse::<f64>().ok());
    let seconds = arg(1).unwrap_or(6.0);
    let repeats = arg(2).unwrap_or(1.0).max(1.0) as usize;
    let (room, bulk) = bulk_stream();
    println!(
        "paced link over loopback TCP, {} {}; {} strokes, {:.1} MB to send; {repeats} runs of {seconds} s each",
        std::env::consts::ARCH,
        std::env::consts::OS,
        room.len(),
        bulk.len() as f64 / 1e6
    );
    println!(
        "cursor delay in ms after the first second; each figure is the middle run, with the least and the most over the runs"
    );
    let rules: [(&str, Rule); 3] = [
        ("20 ms rule", Rule::Drain(Duration::from_millis(20))),
        ("fixed 64 KiB", Rule::Fixed(64 * 1024)),
        ("fixed 1 MiB", Rule::Fixed(1024 * 1024)),
    ];
    for mbit in [1.0, 5.0, 20.0] {
        for (name, rule) in &rules {
            let runs: Vec<Run> = (0..repeats)
                .map(|_| run(*rule, mbit, seconds, &bulk))
                .collect();
            for r in &runs {
                assert_eq!(r.bad_objects, 0, "an object did not survive the trip");
            }
            let stats: Vec<[f64; 4]> = runs.iter().map(summary).collect();
            let show = |i: usize| {
                let (mid, least, most) = over_runs(stats.iter().map(|s| s[i]));
                format!("{mid:>6.0} ({least:.0} to {most:.0})")
            };
            let (system, _, system_most) =
                over_runs(runs.iter().map(|r| r.in_system as f64 / 1024.0));
            let (estimate, _, _) = over_runs(runs.iter().map(|r| r.estimate / 1e6));
            println!(
                "{mbit:>5} Mbit/s  {name:<13} median {}   p95 {}   max {}   first second {}   cursors {:>4}   objects {:>6}   limit {:>5.1} KiB   estimate {estimate:>5.2} Mbit/s   held by the system {system:>6.1} KiB (most {system_most:.1})   buffers given: send {} B, relay {} B",
                show(0),
                show(1),
                show(2),
                show(3),
                runs[0].cursors.len(),
                runs[0].objects,
                runs[0].limit as f64 / 1024.0,
                runs[0].send_buffer,
                runs[0].relay_buffer
            );
        }
    }
}
