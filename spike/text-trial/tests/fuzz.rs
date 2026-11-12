//! Bytes from outside must never bring the server down, and should not change the text behind its back. Each
//! candidate is given damaged updates and damaged whole documents. A panic is counted, not passed on, so that one
//! run reports the whole picture. This test is alone in its file because it silences the panic message for its
//! process.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use text_trial::{Am, Candidate, Loro, Rng, Yrs, typed};

const SEEDS: [u64; 2] = [1, 2];
/// Damaged updates and damaged whole documents, per seed.
const DAMAGED: usize = 2_500;
const DAMAGED_SNAPSHOTS: usize = 500;

/// Where each panic happened, with how often.
static PANICS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

#[derive(Debug, Default, PartialEq)]
struct Tally {
    /// The library said "refused".
    refused: usize,
    /// The library said "ok" and the text is what the undamaged update gives.
    ok_as_sent: usize,
    /// The library said "ok" and the text did not change (the update was parked, or was empty).
    ok_no_change: usize,
    /// The library said "ok" and the text is something else: the damage went in unnoticed.
    ok_altered: usize,
    panicked: usize,
    /// The text changed although the update was refused.
    changed_on_refusal: usize,
    /// Damaged whole documents: loaded, refused, panicked.
    snapshots: [usize; 3],
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.refused += other.refused;
        self.ok_as_sent += other.ok_as_sent;
        self.ok_no_change += other.ok_no_change;
        self.ok_altered += other.ok_altered;
        self.panicked += other.panicked;
        self.changed_on_refusal += other.changed_on_refusal;
        for i in 0..3 {
            self.snapshots[i] += other.snapshots[i];
        }
    }
}

/// Returns the damaged bytes, or `None` when the damage happened to leave them as they were.
fn damage(rng: &mut Rng, good: &[u8]) -> Option<Vec<u8>> {
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
    (b != good).then_some(b)
}

fn fuzz<C: Candidate>(seed: u64) -> Tally {
    let mut rng = Rng(seed);
    // The server holds 40 characters. Each good update is one more edit by the same client (a keystroke at some
    // place, or a deletion), and applies on its own to the server's document.
    let (mut author, _) = typed::<C>(1, 40);
    let base = C::snapshot(&mut author);
    let mut good: Vec<(Vec<u8>, String)> = Vec::new();
    for i in 0..40 {
        let mut one = C::load(1, &base).unwrap();
        let update = if i == 0 {
            C::delete(&mut one, 5, 3)
        } else {
            C::insert(&mut one, i, "Q")
        };
        good.push((update, C::text(&mut one)));
    }

    let mut tally = Tally::default();
    let mut done = 0;
    while done < DAMAGED {
        let (update, as_sent) = &good[rng.below(good.len())];
        let Some(bytes) = damage(&mut rng, update) else {
            continue;
        };
        done += 1;
        // A new server document for every damaged update, so each outcome stands on its own.
        let mut server = C::load(100, &base).unwrap();
        let before = C::text(&mut server);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let result = C::apply(&mut server, &bytes);
            (result, C::text(&mut server))
        }));
        match outcome {
            Ok((Ok(()), after)) if after == *as_sent => tally.ok_as_sent += 1,
            Ok((Ok(()), after)) if after == before => tally.ok_no_change += 1,
            Ok((Ok(()), _)) => tally.ok_altered += 1,
            Ok((Err(_), after)) => {
                tally.refused += 1;
                if after != before {
                    tally.changed_on_refusal += 1;
                }
            }
            Err(_) => tally.panicked += 1,
        }
    }
    // A whole document from storage or from a client, damaged, is loaded or refused.
    let mut done = 0;
    while done < DAMAGED_SNAPSHOTS {
        let Some(bytes) = damage(&mut rng, &base) else {
            continue;
        };
        done += 1;
        match catch_unwind(AssertUnwindSafe(|| C::load(100, &bytes).map(|_| ()))) {
            Ok(Ok(())) => tally.snapshots[0] += 1,
            Ok(Err(_)) => tally.snapshots[1] += 1,
            Err(_) => tally.snapshots[2] += 1,
        }
    }
    tally
}

fn all_seeds<C: Candidate>() -> (Tally, BTreeMap<String, usize>) {
    let mut total = Tally::default();
    for seed in SEEDS {
        total.add(&fuzz::<C>(seed));
    }
    (total, std::mem::take(&mut *PANICS.lock().unwrap()))
}

#[test]
fn what_each_library_does_with_damaged_bytes() {
    std::panic::set_hook(Box::new(|info| {
        let place = info
            .location()
            .map(|l| {
                let file = l.file();
                let short = file.rfind("/src/").map_or(file, |i| &file[i + 1..]);
                format!("{short}:{}", l.line())
            })
            .unwrap_or_default();
        *PANICS.lock().unwrap().entry(place).or_default() += 1;
    }));
    let (loro, loro_panics) = all_seeds::<Loro>();
    let (yrs, yrs_panics) = all_seeds::<Yrs>();
    let (am, am_panics) = all_seeds::<Am>();
    let _ = std::panic::take_hook();

    let updates = DAMAGED * SEEDS.len();
    let documents = DAMAGED_SNAPSHOTS * SEEDS.len();
    for (name, t, panics) in [
        (Loro::NAME, &loro, &loro_panics),
        (Yrs::NAME, &yrs, &yrs_panics),
        (Am::NAME, &am, &am_panics),
    ] {
        println!("{name}: {t:?}");
        for (place, count) in panics {
            println!("    panic at {place}: {count}");
        }
        let answered = t.refused + t.ok_as_sent + t.ok_no_change + t.ok_altered + t.panicked;
        assert_eq!(answered, updates);
        assert_eq!(t.snapshots.iter().sum::<usize>(), documents);
        assert_eq!(
            t.changed_on_refusal, 0,
            "{name} changed its text on a refused update"
        );
        assert!(t.refused > updates / 4, "{name}: the damage was too gentle");
    }

    // Loro never panics, and never lets damage change the text: an update it accepts either gives the text that
    // was sent or changes nothing.
    assert_eq!(
        (loro.panicked, loro.snapshots[2], loro.ok_altered),
        (0, 0, 0),
        "loro: {loro:?}"
    );
    // yrs 0.28.1 and Automerge 0.12.0 each panic on a small share, and each accept some damaged updates that
    // change the text to something that was not sent. So with either, the adapter has to catch a panic around
    // every call that takes outside bytes, and damage in transit is not detected by the library. If one of
    // these stops failing, the library has changed: read the results file again.
    for (name, t) in [(Yrs::NAME, &yrs), (Am::NAME, &am)] {
        let panics = t.panicked + t.snapshots[2];
        assert!(
            panics > 0,
            "{name} no longer panics on damaged bytes: {t:?}"
        );
        assert!(
            t.ok_altered > 0,
            "{name} no longer accepts damaged updates that change the text: {t:?}"
        );
    }
}
