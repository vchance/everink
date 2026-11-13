//! Timing run for spike question 4: what does cutting a large shape cost?
//!
//! Run with `cargo run --release --example bench_cut`. For each number of levels between the shape and the
//! camera it cuts 200 random curves that pass through the window, and reports the time for one curve (median,
//! 95th percentile and the slowest) against the budget of 2 ms a frame. It also times the outline of a stroke.

use std::hint::black_box;
use std::time::Instant;

use everink_core::Axis;
use everink_core::cut::{BigPt, cut_quad};
use everink_core::outline::{InkPoint, build};

const CURVES: usize = 200;
const REPEATS: usize = 5;
const BUDGET_US: f64 = 2000.0;
/// The window's half-side: 2,048 camera-level units, in steps of 2^-40.
const HALF: i64 = 2048 << 40;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A signed number of `bits` bits, as an axis value.
    fn big(&mut self, bits: u32) -> Axis {
        let bytes = bits.div_ceil(8) as usize + 1;
        let mut v: Vec<u8> = (0..bytes).map(|_| self.next() as u8).collect();
        // The first byte is the sign: all ones or all zeros.
        v[0] = if self.next() & 1 == 1 { 0xff } else { 0 };
        Axis::from_signed_bytes_be(&v)
    }
}

/// A curve `levels` levels above the window that passes through a point of the window at its middle.
fn curve(rng: &mut Rng, levels: u32) -> [BigPt; 3] {
    let bits = 10 * levels + 40 + 12;
    let w = |rng: &mut Rng| Axis::from_i128(i128::from(rng.next() as i64 >> 14));
    let (p1x, p1y, p2x, p2y) = (rng.big(bits), rng.big(bits), rng.big(bits), rng.big(bits));
    // p0 = 4 w - 2 p1 - p2, so that the curve's middle is w.
    let first = |w: Axis, p1: &Axis, p2: &Axis| w.shl(2).sub(&p1.shl(1)).sub(p2);
    let p0 = BigPt {
        x: first(w(rng), &p1x, &p2x),
        y: first(w(rng), &p1y, &p2y),
    };
    [p0, BigPt { x: p1x, y: p1y }, BigPt { x: p2x, y: p2y }]
}

/// The part of a curve from a quarter of the way along to its end. A curve built to pass through the window at
/// its middle then crosses it a third of the way along this part, which is the ordinary case: the first halving
/// leaves the crossing in one half only.
fn last_three_quarters(c: &[BigPt; 3]) -> [BigPt; 3] {
    // de Casteljau at 1/4: (3 a + b) / 4.
    let q = |a: &Axis, b: &Axis| a.shl(1).add(a).add(b).shr_floor(2);
    let quarter = |a: &BigPt, b: &BigPt| BigPt {
        x: q(&a.x, &b.x),
        y: q(&a.y, &b.y),
    };
    let (ab, bc) = (quarter(&c[0], &c[1]), quarter(&c[1], &c[2]));
    [quarter(&ab, &bc), bc, c[2].clone()]
}

fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        v[v.len() / 2],
        v[(v.len() * 95 / 100).min(v.len() - 1)],
        v[v.len() - 1],
    )
}

fn main() {
    println!(
        "everink-core cut and outline, {} {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    for (title, typical) in [
        (
            "crossing a third of the way along (the ordinary case)",
            true,
        ),
        (
            "crossing exactly at the middle (the worst case: both halves reach the window)",
            false,
        ),
    ] {
        println!("\none curve cut to the window, {title}");
        println!("{CURVES} curves per row, each timed {REPEATS} times and its fastest kept");
        println!(
            "{:>7} | {:>10} {:>10} {:>10} | {:>9} {:>7} | {:>14}",
            "levels", "median us", "p95 us", "slowest us", "halvings", "pieces", "curves in 2 ms"
        );
        for levels in [1u32, 3, 10, 30, 100, 300, 1000] {
            let mut rng = Rng(u64::from(levels));
            let curves: Vec<[BigPt; 3]> = (0..CURVES)
                .map(|_| {
                    let c = curve(&mut rng, levels);
                    if typical { last_three_quarters(&c) } else { c }
                })
                .collect();
            let mut times = Vec::with_capacity(CURVES);
            let (mut halvings, mut pieces) = (0u64, 0usize);
            for c in &curves {
                let mut best = f64::INFINITY;
                for _ in 0..REPEATS {
                    let input = c.clone();
                    let mut out = Vec::new();
                    let t = Instant::now();
                    let stats = cut_quad(black_box(input), HALF, &mut out);
                    let us = t.elapsed().as_secs_f64() * 1e6;
                    if us < best {
                        best = us;
                    }
                    halvings += u64::from(stats.halvings);
                    pieces += out.len();
                    black_box(out);
                }
                times.push(best);
            }
            let runs = (CURVES * REPEATS) as f64;
            let (med, p95, max) = spread(times);
            println!(
                "{levels:>7} | {med:>10.1} {p95:>10.1} {max:>10.1} | {:>9.0} {:>7.1} | {:>14.1}",
                halvings as f64 / runs,
                pieces as f64 / runs,
                BUDGET_US / med
            );
        }
    }

    println!(
        "\nthe outline of one stroke of 200 points (the same random walk each time), 2,000 builds"
    );
    let mut rng = Rng(9);
    let (mut x, mut y) = (0i32, 0i32);
    let points: Vec<InkPoint> = (0..200)
        .map(|_| {
            x += (rng.next() % 801) as i32 - 400;
            y += (rng.next() % 801) as i32 - 400;
            InkPoint {
                x,
                y,
                pressure: 1 + (rng.next() % 255) as u8,
            }
        })
        .collect();
    let mut times = Vec::with_capacity(2000);
    let mut segs = 0;
    for _ in 0..2000 {
        let t = Instant::now();
        let outline = build(black_box(&points), 3 << 16).expect("a stroke");
        times.push(t.elapsed().as_secs_f64() * 1e6);
        segs = outline.segs.len();
        black_box(outline);
    }
    let (med, p95, max) = spread(times);
    println!(
        "{:<28} {med:>8.1} us median (p95 {p95:.1}, slowest {max:.1}); {segs} segments",
        "runs, smoothed sides"
    );
}
