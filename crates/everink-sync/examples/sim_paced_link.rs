//! Cursor delay during a bulk load, in the model of `everink_sync::link_model` (made-up time; the same on every
//! machine). Run with `cargo run --release --example sim_paced_link`.
//!
//! For each connection speed it prints how long another person's cursor messages wait while bulk data is being
//! sent for 30 seconds without a pause, under the design's 20 ms rule counted two ways, a 40 ms rule, and two
//! fixed limits that stand for no pacing: 64 KiB (the design's earlier fixed limit) and 1 MiB (a send buffer
//! left to the operating system).

use std::time::Duration;

use everink_sync::link_model::{Queues, Report, run};
use everink_sync::pacing::{FIRST_LIMIT, MIN_LIMIT, Rule};

const RUN: Duration = Duration::from_secs(30);
const SPEEDS: [f64; 9] = [0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];

fn load(rule: Rule, first_limit: usize, queues: Queues, mbit: f64) -> Report {
    // More than the connection can send in the run, so the load never ends before the run does.
    let bulk = (mbit * 1e6 / 8.0 * RUN.as_secs_f64() * 1.5) as usize;
    run(rule, first_limit, queues, mbit * 1e6, bulk, RUN)
}

fn main() {
    let ms20 = Rule::Drain(Duration::from_millis(20));
    let ms40 = Rule::Drain(Duration::from_millis(40));
    let columns: [(&str, Rule, Queues); 5] = [
        ("20 ms, one queue", ms20, Queues::One),
        ("20 ms, two queues", ms20, Queues::Two),
        ("40 ms, two queues", ms40, Queues::Two),
        ("fixed 64 KiB", Rule::Fixed(64 * 1024), Queues::One),
        ("fixed 1 MiB", Rule::Fixed(1024 * 1024), Queues::One),
    ];
    println!(
        "cursor delay during a bulk load that lasts the whole run, model link, 30 seconds each"
    );
    println!(
        "median / 95th percentile, in ms, after the first second; then the limit on unsent bytes\n"
    );
    print!("{:>7} ", "Mbit/s");
    for (name, _, _) in &columns {
        print!("| {name:^26} ");
    }
    println!();
    for mbit in SPEEDS {
        print!("{mbit:>7} ");
        for (_, rule, queues) in &columns {
            let r = load(*rule, FIRST_LIMIT, *queues, mbit);
            // The first second is the start-up, shown separately below.
            let steady = Report {
                delays: r.delays[r.delays.len().min(30)..].to_vec(),
                ..r.clone()
            };
            let (med, p95, _) = steady.delay_ms();
            print!(
                "| {med:6.0} /{p95:6.0} {:>7.1} KiB ",
                r.limit as f64 / 1024.0
            );
        }
        println!();
    }

    println!(
        "\nthe start of a load under the 20 ms rule (two queues): the worst cursor delay in the whole run,"
    );
    println!("with the design's starting limit of 16 KiB and with a starting limit of 4 KiB");
    println!(
        "{:>7} | {:>14} | {:>13} | {:>22}",
        "Mbit/s", "start 16 KiB", "start 4 KiB", "limit settles after"
    );
    for mbit in SPEEDS {
        let a = load(ms20, FIRST_LIMIT, Queues::Two, mbit);
        let b = load(ms20, MIN_LIMIT, Queues::Two, mbit);
        println!(
            "{mbit:>7} | {:>11.0} ms | {:>10.0} ms | {:>19} ms",
            a.delay_ms().2,
            b.delay_ms().2,
            a.settled_after.map_or(-1, |d| d.as_millis() as i64)
        );
    }
}
