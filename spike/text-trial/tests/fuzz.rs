//! Bytes from outside must never bring the server down. Each candidate is given damaged updates and random bytes
//! and must answer every one with "applied" or "refused". A panic is counted, not passed on, so that one run
//! reports the whole picture. This test is alone in its file because it silences the panic message for its
//! process.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use text_trial::{Am, Candidate, Loro, Rng, Yrs, typed};

const DAMAGED: usize = 4_000;
const DAMAGED_SNAPSHOTS: usize = 1_000;

/// Where each panic happened, with how often.
static PANICS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

#[derive(Debug, Default, PartialEq)]
struct Tally {
    applied: usize,
    refused: usize,
    panicked: usize,
    /// The text changed although the update was refused.
    changed_on_refusal: usize,
    /// Damaged whole documents: loaded, refused, panicked.
    snapshots: [usize; 3],
}

fn damage(rng: &mut Rng, good: &[u8]) -> Vec<u8> {
    let mut b = good.to_vec();
    match rng.below(5) {
        // One bit.
        0 => {
            let i = rng.below(b.len());
            b[i] ^= 1 << rng.below(8);
        }
        // One byte.
        1 => {
            let i = rng.below(b.len());
            b[i] = rng.next_u64() as u8;
        }
        // Cut short.
        2 => b.truncate(rng.below(b.len())),
        // A length-like byte made large.
        3 => {
            let i = rng.below(b.len());
            b[i] = 0xff;
            if i + 1 < b.len() {
                b[i + 1] = 0xff;
            }
        }
        // Nothing of the original.
        _ => {
            let n = 1 + rng.below(64);
            b = (0..n).map(|_| rng.next_u64() as u8).collect();
        }
    }
    b
}

fn fuzz<C: Candidate>(seed: u64) -> Tally {
    let mut rng = Rng(seed);
    // The server holds 40 characters; the updates are the next 40 keystrokes of the same client and a deletion.
    let (mut author, updates) = typed::<C>(1, 80);
    let mut base = C::new(100);
    for u in &updates[..40] {
        C::apply(&mut base, u).unwrap();
    }
    let base = C::snapshot(&mut base);
    let mut good: Vec<Vec<u8>> = updates[40..].to_vec();
    good.push(C::delete(&mut author, 5, 3));
    let whole = C::snapshot(&mut author);

    let mut tally = Tally::default();
    let mut server = C::load(100, &base).unwrap();
    for i in 0..DAMAGED {
        if i % 25 == 0 {
            server = C::load(100, &base).unwrap();
        }
        let pick = rng.below(good.len());
        let bytes = damage(&mut rng, &good[pick]);
        let before = C::text(&mut server);
        match catch_unwind(AssertUnwindSafe(|| C::apply(&mut server, &bytes))) {
            Ok(Ok(())) => tally.applied += 1,
            Ok(Err(_)) => {
                tally.refused += 1;
                let after = catch_unwind(AssertUnwindSafe(|| C::text(&mut server)));
                if after.ok().as_ref() != Some(&before) {
                    tally.changed_on_refusal += 1;
                    server = C::load(100, &base).unwrap();
                }
            }
            Err(_) => {
                tally.panicked += 1;
                // A document that panicked half-way is not used again.
                server = C::load(100, &base).unwrap();
            }
        }
    }
    // A whole document from storage or from a client, damaged, is loaded or refused.
    for _ in 0..DAMAGED_SNAPSHOTS {
        let bytes = damage(&mut rng, &whole);
        match catch_unwind(AssertUnwindSafe(|| C::load(100, &bytes).map(|_| ()))) {
            Ok(Ok(())) => tally.snapshots[0] += 1,
            Ok(Err(_)) => tally.snapshots[1] += 1,
            Err(_) => tally.snapshots[2] += 1,
        }
    }
    tally
}

#[test]
fn damaged_updates_are_applied_or_refused() {
    std::panic::set_hook(Box::new(|info| {
        let place = info
            .location()
            .map(|l| {
                let file = l.file();
                let short = file.find("/src/").map_or(file, |i| &file[i + 1..]);
                format!("{short}:{}", l.line())
            })
            .unwrap_or_default();
        *PANICS.lock().unwrap().entry(place).or_default() += 1;
    }));
    let loro = fuzz::<Loro>(1);
    let loro_panics = std::mem::take(&mut *PANICS.lock().unwrap());
    let yrs = fuzz::<Yrs>(1);
    let yrs_panics = std::mem::take(&mut *PANICS.lock().unwrap());
    let am = fuzz::<Am>(1);
    let am_panics = std::mem::take(&mut *PANICS.lock().unwrap());
    let _ = std::panic::take_hook();
    for (name, t, panics) in [
        (Loro::NAME, &loro, &loro_panics),
        (Yrs::NAME, &yrs, &yrs_panics),
        (Am::NAME, &am, &am_panics),
    ] {
        println!("{name}: {t:?}");
        for (place, count) in panics {
            println!("    panic at {place}: {count}");
        }
        assert_eq!(t.applied + t.refused + t.panicked, DAMAGED);
        assert_eq!(t.snapshots.iter().sum::<usize>(), DAMAGED_SNAPSHOTS);
        assert_eq!(
            t.changed_on_refusal, 0,
            "{name} changed its text on a refused update"
        );
        assert!(t.refused > DAMAGED / 4, "{name}: the damage was too gentle");
    }
    // Loro answers every damaged update and document without a panic.
    assert_eq!(
        (loro.panicked, loro.snapshots[2]),
        (0, 0),
        "loro panicked: {loro:?}"
    );
    // yrs 0.28.1 and Automerge 0.12.0 each panic on a small share of them, each at one place. The adapter's rule
    // "must not panic" therefore needs the panic caught around every call into either. If one of these stops
    // failing, the library has changed: read the results file again.
    for (name, t) in [(Yrs::NAME, &yrs), (Am::NAME, &am)] {
        let all = t.panicked + t.snapshots[2];
        assert!(all > 0, "{name} no longer panics on damaged bytes: {t:?}");
        assert!(
            all < (DAMAGED + DAMAGED_SNAPSHOTS) / 50,
            "{name} panics far more often than recorded: {t:?}"
        );
    }
}
