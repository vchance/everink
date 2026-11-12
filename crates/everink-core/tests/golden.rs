//! Cross-platform check: the same inputs must give byte-identical results on every target.
//!
//! The file `tests/golden/positions.txt` was written on one machine (aarch64 macOS). This test rebuilds every line
//! from a fixed seed and compares. Run on another architecture, a difference means position arithmetic, the
//! canonical bytes or the float conversion depends on the machine. Set `UPDATE_GOLDEN=1` to rewrite the file.

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use common::{Rng, deep_position, hex};
use everink_core::{Level, Rel};

fn rel_bits(r: Rel) -> String {
    match r {
        Rel::Near(v) => format!("{:016x}", v.to_bits()),
        Rel::Far(s) => format!("far{s:+}"),
    }
}

fn build() -> String {
    let mut out = String::new();
    let mut rng = Rng(0x00e7_e121_6b00_0001);
    // Random positions over the whole depth range: bytes, a level change each way, and a difference as floats.
    for i in 0..400 {
        let a = rng.position(1024, 2_000);
        let b = rng.position(1024, 2_000);
        let unit = Level(a.level.0.max(b.level.0) - (i % 7));
        let d = a.sub(&b);
        let [fx, fy] = d.to_f64(unit);
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
    // Pairs that are close together at depth: the case the renderer meets, where the float is small and exact
    // bits matter.
    for depth in [0, 1, 3, 30, 100, 300, 1000] {
        for _ in 0..40 {
            let a = deep_position(&mut rng, depth);
            let mut b = a.clone();
            b.x = b.x.add(&everink_core::Axis::from_i128(
                (rng.next() as i64 >> 3) as i128,
            ));
            b.y = b.y.sub(&everink_core::Axis::from_i128(
                (rng.next() as i64 >> 9) as i128,
            ));
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
    out
}

#[test]
fn results_match_the_golden_file_byte_for_byte() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/positions.txt");
    let built = build();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
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
