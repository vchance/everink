//! Cross-platform check: the same inputs must give byte-identical results on every target.
//!
//! The file `tests/golden/positions.txt` was written on one machine (aarch64 macOS). This test rebuilds every line
//! from a fixed seed and compares. On the machine that wrote the file this proves nothing; its value is on another
//! architecture, where a difference means position arithmetic, the canonical bytes or the float conversion
//! depends on the machine. CI runs it on x86-64.
//!
//! `UPDATE_GOLDEN=1` rewrites the file. It is refused when `CI` is set, so a CI run can never pass by rewriting.

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use common::{Rng, deep_position, hex};
use everink_core::{Axis, Delta, Level, Rel};
use num_bigint::BigInt;
use num_traits::Signed;

fn rel_bits(r: Rel) -> String {
    match r {
        Rel::Near(v) => format!("{:016x}", v.to_bits()),
        Rel::Far(s) => format!("far{s:+}"),
    }
}

fn build() -> String {
    let mut out = String::new();
    let mut rng = Rng(0x00e7_e121_6b00_0001);

    // r: random positions over the whole depth range: canonical bytes, a floor to a coarser level, and their
    // difference as floats. Most differences here are too large to be a float and come out Far.
    for i in 0..400 {
        let a = rng.position(1024, 2_000);
        let b = rng.position(1024, 2_000);
        let unit = Level(a.level.0.max(b.level.0) - (i % 7));
        let [fx, fy] = a.sub(&b).to_f64(unit);
        let coarser = a.at_coarser_floor(Level(a.level.0 - (i % 5)));
        writeln!(
            out,
            "r {} {} {} {} {} {}",
            hex(&a.encode()),
            hex(&b.encode()),
            unit.0,
            rel_bits(fx),
            rel_bits(fy),
            hex(&coarser.encode())
        )
        .unwrap();
    }

    // n: pairs close together at depth, the case the renderer meets. The difference fits 128 bits, so this is the
    // inline rounding path.
    for depth in [0, 1, 3, 30, 100, 300, 1000] {
        for _ in 0..40 {
            let a = deep_position(&mut rng, depth);
            let mut b = a.clone();
            b.x = b.x.add(&Axis::from_i128((rng.next() as i64 >> 3) as i128));
            b.y = b.y.sub(&Axis::from_i128((rng.next() as i64 >> 9) as i128));
            let [fx, fy] = b.sub(&a).to_f64(Level(depth));
            writeln!(
                out,
                "n {} {} {} {}",
                depth,
                hex(&b.encode()),
                rel_bits(fx),
                rel_bits(fy)
            )
            .unwrap();
        }
    }

    // b: differences of 129 to 1,000 bits that stay inside the float range. This is the big-integer rounding path
    // producing a finite float, which the lines above hardly reach.
    for _ in 0..400 {
        let bits = 129 + rng.below(872);
        let d = Delta {
            level: Level(0),
            dx: Axis::from_big(rng.big(bits)),
            dy: Axis::from_big(rng.big(bits)),
        };
        let unit = Level(rng.below(9) as i32 - 4);
        let [fx, fy] = d.to_f64(unit);
        writeln!(
            out,
            "b {} {} {} {}",
            bits,
            unit.0,
            rel_bits(fx),
            rel_bits(fy)
        )
        .unwrap();
    }

    // t: exact 53-bit ties and one step either side of a tie, on both rounding paths, both signs. The tie is built
    // from the value's real bit length: keep 53 bits, set the next one, clear the rest.
    for i in 0..300 {
        let bits = if i % 2 == 0 {
            60 + rng.below(60)
        } else {
            140 + rng.below(700)
        };
        let v = rng.big(bits);
        let low = v.bits().max(60) - 53;
        let sign = if v.is_negative() { -1 } else { 1 };
        let tie = (((v.abs() >> low) << low) + (BigInt::from(1) << (low - 1))) * sign;
        for step in [-1, 0, 1] {
            let d = Delta {
                level: Level(0),
                dx: Axis::from_big(&tie + BigInt::from(step)),
                dy: Axis::ZERO,
            };
            writeln!(
                out,
                "t {} {} {}",
                low,
                step,
                rel_bits(d.to_f64(Level(0))[0])
            )
            .unwrap();
        }
    }

    // e: the ends of the range. 2^1000 units and above is Far; below 2^-1000 units is zero. With the unit level
    // equal to the level a step is 2^-54 units.
    let one = BigInt::from(1);
    let top = one.clone() << 1054u32;
    let cases: [(&str, BigInt, i32); 8] = [
        ("top", top.clone(), 0),
        ("top-1", &top - &one, 0),
        ("under", &top - (one.clone() << 1001u32), 0),
        ("-top", -top.clone(), 0),
        ("2^-1000", BigInt::from(16), -95),
        ("15*2^-1004", BigInt::from(15), -95),
        ("2^-1004", one.clone(), -95),
        ("round-up", (one.clone() << 54u32) - &one, -100),
    ];
    for (name, d, unit) in cases {
        let delta = Delta {
            level: Level(0),
            dx: Axis::from_big(d),
            dy: Axis::ZERO,
        };
        writeln!(out, "e {} {}", name, rel_bits(delta.to_f64(Level(unit))[0])).unwrap();
    }
    out
}

#[test]
fn results_match_the_golden_file_byte_for_byte() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/positions.txt");
    let built = build();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        assert!(
            std::env::var_os("CI").is_none(),
            "the golden file is never rewritten in CI"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &built).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("golden file present; run with UPDATE_GOLDEN=1 to create it");
    assert_eq!(
        built.lines().count(),
        golden.lines().count(),
        "line count differs"
    );
    for (n, (b, g)) in built.lines().zip(golden.lines()).enumerate() {
        assert_eq!(b, g, "line {} differs from the golden file", n + 1);
    }
}

#[test]
fn the_golden_file_exercises_the_float_conversion() {
    // Guards against the file going back to being almost all Far: count what the lines actually contain.
    let built = build();
    // A finite, non-zero float: sixteen hex digits that are not all zero (zero and minus zero are excluded, so a
    // conversion that went back to returning zeros would fail here).
    let float = |s: &str| {
        s.len() == 16
            && s.chars().all(|c| c.is_ascii_hexdigit())
            && s != "0000000000000000"
            && s != "8000000000000000"
    };
    let near_b = built
        .lines()
        .filter(|l| l.starts_with("b "))
        .flat_map(|l| l.split(' ').skip(3))
        .filter(|s| float(s))
        .count();
    let near_t = built
        .lines()
        .filter(|l| l.starts_with("t "))
        .filter(|l| float(l.rsplit(' ').next().unwrap()))
        .count();
    assert_eq!(near_b, 800, "every big-path difference is a finite float");
    assert_eq!(near_t, 900, "every tie case is a finite float");
}
