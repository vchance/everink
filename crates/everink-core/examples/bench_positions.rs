//! Timing run for the spike question "What does exact position arithmetic cost?".
//!
//! Run with `cargo run --release --example bench_positions`. It prints one table. The numbers are wall-clock
//! medians of several runs on whatever machine runs it; `docs/spike/results-1-positions.md` records one such run
//! with the machine named.

use std::hint::black_box;
use std::time::Instant;

use everink_core::{Axis, Camera, DepthCap, Level, Position, Rel};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// `count` objects scattered within a few screens of a point `depth` levels down, as a view at that depth holds.
fn scene(rng: &mut Rng, depth: i32, count: usize) -> (Position, Vec<Position>) {
    let origin = Position {
        level: Level(0),
        x: Axis::from_i128((rng.next() >> 1) as i128),
        y: Axis::from_i128((rng.next() >> 1) as i128),
    };
    let mut center = origin.at_level(Level(depth));
    center.x = center.x.add(&Axis::from_i128(rng.next() as i128));
    center.y = center.y.add(&Axis::from_i128(rng.next() as i128));
    let objects = (0..count)
        .map(|_| {
            let mut p = center.clone();
            // Within about +-8,000 units: a few screens at any in-level zoom.
            p.x = p.x.add(&Axis::from_i128(
                (rng.next() as i64 >> 37) as i128 * (1 << 40),
            ));
            p.y = p.y.add(&Axis::from_i128(
                (rng.next() as i64 >> 37) as i128 * (1 << 40),
            ));
            p
        })
        .collect();
    (center, objects)
}

/// Median nanoseconds per item over `runs` runs of `f`, where one run handles `items` items.
fn median_ns_per_item(runs: usize, items: usize, mut f: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_nanos() as f64 / items as f64
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn main() {
    const N: usize = 10_000;
    let mut rng = Rng(42);
    println!("objects per run: {N}; times are medians of 31 runs");
    println!();
    println!(
        "| depth (levels) | position bytes | subtract (ns) | subtract + to float (ns) | re-base {N} objects (ms) | \
         one frame, {N} objects (ms) | encode (ns) | decode (ns) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for depth in [0, 1, 10, 100, 1000] {
        let (center, objects) = scene(&mut rng, depth, N);
        let cam = Camera::new(center.clone(), 100.0, [1920.0, 1080.0]);
        let bytes: Vec<Vec<u8>> = objects.iter().map(Position::encode).collect();
        let mean_bytes = bytes.iter().map(Vec::len).sum::<usize>() / N;

        let sub = median_ns_per_item(31, N, || {
            for p in &objects {
                black_box(black_box(p).sub(black_box(&center)));
            }
        });
        let sub_f64 = median_ns_per_item(31, N, || {
            for p in &objects {
                black_box(black_box(p).sub(black_box(&center)).to_f64(Level(depth)));
            }
        });

        // A re-base: every object's offset from the render origin, exactly, then as a float. This is what runs when
        // the camera has moved far from the origin the cached floats were measured from.
        let mut cached: Vec<[f64; 2]> = vec![[0.0; 2]; N];
        let rebase = median_ns_per_item(31, 1, || {
            for (slot, p) in cached.iter_mut().zip(&objects) {
                let [x, y] = p.sub(&cam.center).to_f64(cam.level);
                *slot = [rel(x), rel(y)];
            }
            black_box(&cached);
        }) / 1e6;

        // One frame after a pan: one exact subtraction (camera center against the render origin), then a few float
        // operations per object.
        let mut moved = cam.clone();
        moved.pan_by_px(300.0, -120.0);
        let mut screen: Vec<[f32; 2]> = vec![[0.0; 2]; N];
        let frame = median_ns_per_item(31, 1, || {
            let [cx, cy] = black_box(&moved).center.sub(&cam.center).to_f64(cam.level);
            let (cx, cy) = (rel(cx), rel(cy));
            for (out, c) in screen.iter_mut().zip(&cached) {
                *out = [
                    ((c[0] - cx) * moved.zoom) as f32,
                    ((c[1] - cy) * moved.zoom) as f32,
                ];
            }
            black_box(&screen);
        }) / 1e6;

        let encode = median_ns_per_item(31, N, || {
            for p in &objects {
                black_box(black_box(p).encode());
            }
        });
        let decode = median_ns_per_item(31, N, || {
            for b in &bytes {
                black_box(Position::decode(black_box(b), &DepthCap::DEFAULT).unwrap());
            }
        });

        println!(
            "| {depth} | {mean_bytes} | {sub:.1} | {sub_f64:.1} | {rebase:.3} | {frame:.4} | {encode:.0} | {decode:.0} |"
        );
    }
}

fn rel(r: Rel) -> f64 {
    match r {
        Rel::Near(v) => v,
        Rel::Far(_) => f64::INFINITY,
    }
}
