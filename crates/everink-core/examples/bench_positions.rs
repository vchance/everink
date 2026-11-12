//! Timing run for the spike question "What does exact position arithmetic cost?".
//!
//! Run with `cargo run --release --example bench_positions`. It prints two tables: the median of 51 runs, and the
//! spread (minimum, median, 95th percentile) of the two figures that are compared with a per-frame budget. The
//! numbers are wall-clock times on whatever machine runs it; `docs/spike/results-1-positions.md` records one run
//! with the machine named. A CI runner is not a reference machine.

use std::hint::black_box;
use std::time::Instant;

use everink_core::{Axis, Camera, DepthCap, Level, Position, Rel};

const N: usize = 10_000;
const RUNS: usize = 51;

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

/// Minimum, median and 95th percentile over `RUNS` runs of `f`, in nanoseconds divided by `per`.
fn spread(per: usize, mut f: impl FnMut()) -> [f64; 3] {
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_nanos() as f64 / per as f64
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    [times[0], times[RUNS / 2], times[RUNS * 95 / 100]]
}

fn rel(r: Rel) -> f64 {
    match r {
        Rel::Near(v) => v,
        Rel::Far(_) => f64::INFINITY,
    }
}

fn main() {
    let mut rng = Rng(42);
    let mut rows = Vec::new();
    let mut spreads = Vec::new();
    for depth in [0, 1, 10, 100, 1000] {
        let (center, objects) = scene(&mut rng, depth, N);
        let cam = Camera::new(center.clone(), 100.0, [1920.0, 1080.0]).expect("a valid camera");
        let bytes: Vec<Vec<u8>> = objects.iter().map(Position::encode).collect();
        let mean_bytes = bytes.iter().map(Vec::len).sum::<usize>() / N;

        let sub = spread(N, || {
            for p in &objects {
                black_box(black_box(p).sub(black_box(&center)));
            }
        });
        let sub_f64 = spread(N, || {
            for p in &objects {
                black_box(black_box(p).sub(black_box(&center)).to_f64(Level(depth)));
            }
        });

        // A re-base: every object's offset from the render origin, exactly, then as a float. This is what runs when
        // the camera has moved far from the origin the cached floats were measured from.
        let mut cached: Vec<[f64; 2]> = vec![[0.0; 2]; N];
        let rebase = spread(1, || {
            for (slot, p) in cached.iter_mut().zip(&objects) {
                let [x, y] = black_box(p).sub(&cam.center).to_f64(cam.level);
                *slot = [rel(x), rel(y)];
            }
            black_box(&cached);
        });

        // One frame after a pan: one exact subtraction (camera center against the render origin), then per object
        // a subtraction, a multiply by the zoom and a multiply by the object's level scale, in floats. The level
        // scale is 1 here (every object is on the camera's level) but the multiply is done, as the design has it.
        let mut moved = cam.clone();
        moved.pan_by_px(300.0, -120.0).expect("a valid pan");
        let level_scale: Vec<f64> = vec![1.0; N];
        let mut screen: Vec<[f32; 3]> = vec![[0.0; 3]; N];
        let frame = spread(1, || {
            let [cx, cy] = black_box(&moved).center.sub(&cam.center).to_f64(cam.level);
            let (cx, cy) = (rel(cx), rel(cy));
            let zoom = black_box(moved.zoom);
            for ((out, c), s) in screen.iter_mut().zip(&cached).zip(black_box(&level_scale)) {
                *out = [
                    ((c[0] - cx) * zoom) as f32,
                    ((c[1] - cy) * zoom) as f32,
                    (zoom * s) as f32,
                ];
            }
            black_box(&screen);
        });

        let encode = spread(N, || {
            for p in &objects {
                black_box(black_box(p).encode());
            }
        });
        let decode = spread(N, || {
            for b in &bytes {
                black_box(Position::decode(black_box(b), &DepthCap::DEFAULT).unwrap());
            }
        });

        rows.push(format!(
            "| {depth} | {mean_bytes} | {:.1} | {:.1} | {:.3} | {:.4} | {:.0} | {:.0} |",
            sub[1],
            sub_f64[1],
            rebase[1] / 1e6,
            frame[1] / 1e6,
            encode[1],
            decode[1]
        ));
        spreads.push(format!(
            "| {depth} | {:.3} | {:.3} | {:.3} | {:.4} | {:.4} | {:.4} |",
            rebase[0] / 1e6,
            rebase[1] / 1e6,
            rebase[2] / 1e6,
            frame[0] / 1e6,
            frame[1] / 1e6,
            frame[2] / 1e6
        ));
    }

    println!("objects per run: {N}; medians of {RUNS} runs");
    println!();
    println!(
        "| depth (levels) | position bytes | subtract (ns) | subtract + to float (ns) | re-base {N} objects (ms) | \
         one frame, {N} objects (ms) | encode (ns) | decode (ns) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for r in &rows {
        println!("{r}");
    }
    println!();
    println!("spread over {RUNS} runs, in ms");
    println!();
    println!(
        "| depth (levels) | re-base min | re-base median | re-base p95 | frame min | frame median | frame p95 |"
    );
    println!("|---|---|---|---|---|---|---|");
    for s in &spreads {
        println!("{s}");
    }
}
