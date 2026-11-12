//! Size and timing run for spike questions 5 and 6.
//!
//! `cargo run --release --manifest-path spike/text-trial/Cargo.toml` prints one block per candidate. Memory is
//! read from the operating system, so each memory figure is taken in a process of its own that this program
//! starts (`text-trial memory <candidate> <empty|full> <count>`).

use std::hint::black_box;
use std::time::Instant;

use text_trial::{Am, Candidate, Loro, Yrs, typed};

/// A text object in the trial: this many characters, typed one at a time.
const CHARS: usize = 200;
/// Keystrokes applied in the timing runs.
const KEYSTROKES: usize = 1_000;
const RUNS: usize = 7;
const MEMORY_DOCS: usize = 10_000;
const VIEW_DOCS: usize = 5_000;
const MAX_TEXT_CHARS: usize = 50_000;

fn resident_bytes() -> Option<usize> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib: usize = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    Some(kib * 1024)
}

/// Median, minimum and maximum.
fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[0], v[v.len() - 1])
}

fn show(what: &str, unit: &str, v: Vec<f64>) {
    let runs = v.len();
    let (med, min, max) = spread(v);
    println!("  {what:44} {med:9.2} {unit}, median of {runs} (min {min:.2}, max {max:.2})");
}

/// Holds `count` documents and prints how much the process grew per document.
fn memory<C: Candidate>(full: bool, count: usize) {
    let snapshot = {
        let (mut doc, _) = typed::<C>(1, if full { CHARS } else { 0 });
        C::snapshot(&mut doc)
    };
    let before = resident_bytes();
    let docs: Vec<C::Doc> = (0..count)
        .map(|i| C::load(i as u64 + 10, &snapshot).expect("its own snapshot"))
        .collect();
    let after = resident_bytes();
    black_box(&docs);
    match (before, after) {
        (Some(b), Some(a)) => println!("{}", a.saturating_sub(b) / count),
        _ => println!("-"),
    }
}

/// Runs this program again for one memory figure, and returns bytes per document.
fn memory_of(candidate: &str, case: &str) -> Option<usize> {
    let exe = std::env::current_exe().ok()?;
    let out = std::process::Command::new(exe)
        .args(["memory", candidate, case, &MEMORY_DOCS.to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

fn report<C: Candidate>(key: &str) {
    println!("\n== {} ==", C::NAME);

    // Bytes.
    let (mut doc, updates) = typed::<C>(1, KEYSTROKES);
    let sizes: Vec<f64> = updates.iter().map(|u| u.len() as f64).collect();
    let (med, min, max) = spread(sizes);
    println!(
        "  update for one keystroke:                    {med:9.0} bytes median (min {min:.0}, max {max:.0})"
    );
    let (mut small, _) = typed::<C>(1, CHARS);
    let small_snapshot = C::snapshot(&mut small);
    let empty = C::snapshot(&mut C::new(1));
    println!(
        "  whole document as bytes:                     {:9} bytes with {CHARS} characters; {} empty; {} with {KEYSTROKES}",
        small_snapshot.len(),
        empty.len(),
        C::snapshot(&mut doc).len()
    );

    // Memory, each in a process of its own.
    match (memory_of(key, "empty"), memory_of(key, "full")) {
        (Some(e), Some(f)) => println!(
            "  memory per document held:                    {f:9} bytes with {CHARS} characters; {e} empty ({:.1} MB and {:.1} MB for 1,000; measured over {MEMORY_DOCS})",
            f as f64 * 1000.0 / 1e6,
            e as f64 * 1000.0 / 1e6
        ),
        _ => println!("  memory per document held:                    not available (no `ps`)"),
    }

    // Time to apply a keystroke, with and without the server's check.
    let (mut plain, mut checked) = (Vec::new(), Vec::new());
    for _ in 0..RUNS {
        let mut server = C::new(100);
        let t = Instant::now();
        for u in &updates {
            C::apply(&mut server, u).expect("a good update");
        }
        plain.push(t.elapsed().as_secs_f64() * 1e6 / updates.len() as f64);
        assert_eq!(C::len(&mut server), KEYSTROKES);

        let mut server = C::new(100);
        let t = Instant::now();
        for u in &updates {
            C::checked_apply(&mut server, u, MAX_TEXT_CHARS).expect("a good update");
        }
        checked.push(t.elapsed().as_secs_f64() * 1e6 / updates.len() as f64);
        assert_eq!(C::len(&mut server), KEYSTROKES);
    }
    show("apply one keystroke:", "us", plain);
    show("... with the scratch-copy check first:", "us", checked);

    // The same check on a document that stays small: a text object of about 200 characters.
    let mut small_checked = Vec::new();
    for _ in 0..RUNS {
        let mut author = C::load(1, &small_snapshot).expect("its own snapshot");
        let mut server = C::load(100, &small_snapshot).expect("its own snapshot");
        let edits: Vec<Vec<u8>> = (0..200)
            .map(|i| {
                if i % 2 == 0 {
                    C::insert(&mut author, CHARS / 2, "x")
                } else {
                    C::delete(&mut author, CHARS / 2, 1)
                }
            })
            .collect();
        let t = Instant::now();
        for u in &edits {
            C::checked_apply(&mut server, u, MAX_TEXT_CHARS).expect("a good update");
        }
        small_checked.push(t.elapsed().as_secs_f64() * 1e6 / edits.len() as f64);
    }
    show(
        "... the same on a 200-character document:",
        "us",
        small_checked,
    );

    // Opening a view of many text objects: one small document each.
    let mut open = Vec::new();
    for _ in 0..RUNS {
        let t = Instant::now();
        let docs: Vec<C::Doc> = (0..VIEW_DOCS)
            .map(|i| C::load(i as u64 + 10, &small_snapshot).expect("its own snapshot"))
            .collect();
        open.push(t.elapsed().as_secs_f64() * 1e3);
        black_box(docs);
    }
    show(
        &format!("load {VIEW_DOCS} documents of {CHARS} characters:"),
        "ms",
        open,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("memory") {
        let full = args[3] == "full";
        let count: usize = args[4].parse().expect("a count");
        match args[2].as_str() {
            "loro" => memory::<Loro>(full, count),
            "yrs" => memory::<Yrs>(full, count),
            _ => memory::<Am>(full, count),
        }
        return;
    }
    println!(
        "text-library trial, {} {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    report::<Loro>("loro");
    report::<Yrs>("yrs");
    report::<Am>("automerge");
}
