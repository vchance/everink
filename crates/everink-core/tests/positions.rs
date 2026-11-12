//! Tests for the position type: exactness, the canonical bytes, the float conversion and the camera.

mod common;

use common::{Rng, deep_position, hex};
use everink_core::wire::check_cap_bytes;
use everink_core::{Axis, Camera, DecodeError, Delta, DepthCap, Level, Position, Rel};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

const CAP: DepthCap = DepthCap::DEFAULT;

// ---------------------------------------------------------------------------------------------------------------
// Canonical bytes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn encoded_sizes_match_the_design() {
    // Level 0, cell 0: 1 + 9 + 9 bytes.
    let origin = Position::origin(Level(0));
    assert_eq!(origin.encode().len(), 19);

    // Level 0, cells below 256: 1 + 10 + 10 bytes.
    let small = Position {
        level: Level(0),
        x: Axis::from_cell_frac(BigInt::from(200), 7),
        y: Axis::from_cell_frac(BigInt::from(-3), 9),
    };
    assert_eq!(small.encode().len(), 21);

    // Level 100, near the origin: a cell of about 1,000 bits is 125 bytes per axis.
    let cell_1000_bits = (BigInt::from(1) << 999u32) + BigInt::from(12345);
    let deep = Position {
        level: Level(100),
        x: Axis::from_cell_frac(cell_1000_bits.clone(), 1),
        y: Axis::from_cell_frac(-cell_1000_bits, 2),
    };
    assert_eq!(deep.encode().len(), 2 + 2 * (2 + 125 + 8));

    // The default cap: level 1,024 and a 12,288-bit cell is 3,094 bytes.
    let cell_max = (BigInt::from(1) << 12_287u32) + BigInt::from(1);
    let at_cap = Position {
        level: Level(1024),
        x: Axis::from_cell_frac(cell_max.clone(), 0),
        y: Axis::from_cell_frac(-cell_max, u64::MAX),
    };
    let bytes = at_cap.encode();
    assert_eq!(bytes.len(), 3094);
    assert_eq!(Position::decode(&bytes, &CAP).unwrap(), at_cap);
}

#[test]
fn encode_then_decode_is_identity_and_bytes_are_unique() {
    let mut rng = Rng(1);
    for _ in 0..5_000 {
        let p = rng.position(1024, 3_000);
        let bytes = p.encode();
        let q =
            Position::decode(&bytes, &CAP).unwrap_or_else(|e| panic!("{e:?} for {}", hex(&bytes)));
        assert_eq!(p, q);
        assert_eq!(q.encode(), bytes, "one value has one encoding");
        assert_eq!(check_cap_bytes(&bytes, &CAP), Ok(()));
    }
}

#[test]
fn decoder_refuses_what_is_not_canonical_or_beyond_the_cap() {
    let good = Position::origin(Level(0)).encode();
    assert_eq!(
        Position::decode(&good, &CAP),
        Ok(Position::origin(Level(0)))
    );

    // Trailing bytes.
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        Position::decode(&trailing, &CAP),
        Err(DecodeError::TrailingBytes)
    );

    // Truncated.
    assert_eq!(
        Position::decode(&good[..good.len() - 1], &CAP),
        Err(DecodeError::Truncated)
    );
    assert_eq!(Position::decode(&[], &CAP), Err(DecodeError::Truncated));

    // A level written in two bytes where one is enough.
    let mut long_level = vec![0x80, 0x00];
    long_level.extend_from_slice(&good[1..]);
    assert_eq!(
        Position::decode(&long_level, &CAP),
        Err(DecodeError::NotCanonical)
    );

    // Negative zero: len = 0 with the sign bit set.
    let mut neg_zero = good.clone();
    neg_zero[1] = 1;
    assert_eq!(
        Position::decode(&neg_zero, &CAP),
        Err(DecodeError::NotCanonical)
    );

    // A magnitude with a leading zero byte: len = 2, bytes 00 05.
    let mut leading_zero = vec![0u8]; // level 0
    leading_zero.push(2 * 2); // len 2, positive
    leading_zero.extend_from_slice(&[0x00, 0x05]);
    leading_zero.extend_from_slice(&[0; 8]);
    leading_zero.extend_from_slice(&good[10..]); // y axis of the origin
    assert_eq!(
        Position::decode(&leading_zero, &CAP),
        Err(DecodeError::NotCanonical)
    );

    // A level beyond the cap.
    let too_deep = Position::origin(Level(1025)).encode();
    assert_eq!(
        Position::decode(&too_deep, &CAP),
        Err(DecodeError::LevelBeyondCap)
    );
    let too_far_out = Position::origin(Level(-1025)).encode();
    assert_eq!(
        Position::decode(&too_far_out, &CAP),
        Err(DecodeError::LevelBeyondCap)
    );

    // A cell beyond the cap: the header alone is enough to refuse it, before any number is built.
    let huge_len: u64 = 1537; // one byte more than 12,288 bits
    let mut beyond = vec![0u8];
    let mut h = huge_len * 2;
    while h >= 0x80 {
        beyond.push((h as u8) | 0x80);
        h >>= 7;
    }
    beyond.push(h as u8);
    assert_eq!(
        check_cap_bytes(&beyond, &CAP),
        Err(DecodeError::CellBeyondCap)
    );

    // A length that claims more than the address space must not allocate or wrap.
    let absurd = [
        0u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
    ];
    assert_eq!(
        check_cap_bytes(&absurd, &CAP),
        Err(DecodeError::CellBeyondCap)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Exact arithmetic
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn shifts_are_exact_and_floor() {
    let mut rng = Rng(2);
    for _ in 0..20_000 {
        let a = rng.axis(400);
        let k = rng.below(300) as u32;

        // Shifting left agrees with the big-integer reference, whichever representation is used.
        assert_eq!(a.shl(k).to_big(), a.to_big() << k);
        // Left then right gives the value back.
        assert_eq!(a.shl(k).shr_floor(k), a);

        // Right shift rounds toward minus infinity: q * 2^k <= a < (q + 1) * 2^k.
        let q = a.shr_floor(k);
        let lo = q.to_big() << k;
        let hi = (q.to_big() + BigInt::from(1)) << k;
        assert!(
            lo <= a.to_big() && a.to_big() < hi,
            "floor failed for {a:?} >> {k}"
        );
    }
}

#[test]
fn cell_and_frac_are_the_high_and_low_part() {
    let mut rng = Rng(3);
    for _ in 0..20_000 {
        let cell = rng.big(rng.0 % 300);
        let frac = rng.next();
        let a = Axis::from_cell_frac(cell.clone(), frac);
        assert_eq!(a.cell(), cell);
        assert_eq!(a.frac(), frac);
        // The representation is canonical: a value that fits 128 bits is held inline.
        assert_eq!(a.is_small(), i128::try_from(a.to_big()).is_ok());
    }
}

#[test]
fn difference_and_sum_are_exact_across_levels() {
    let mut rng = Rng(4);
    for _ in 0..5_000 {
        let a = rng.position(300, 500);
        let b = rng.position(300, 500);
        let d = a.sub(&b);
        assert_eq!(d.level, a.level.max(b.level));
        // b + (a - b) is the same point as a.
        assert!(b.add_delta(&d).same_point(&a));
        // a - a is zero.
        let zero = a.sub(&a);
        assert!(zero.dx.is_zero() && zero.dy.is_zero());
    }
}

#[test]
fn changing_level_keeps_the_point() {
    let mut rng = Rng(5);
    for _ in 0..5_000 {
        let p = rng.position(200, 300);
        let finer = Level(p.level.0 + rng.below(40) as i32);
        let q = p.at_level(finer);
        assert!(q.same_point(&p));
        assert_eq!(
            q.at_coarser_floor(p.level),
            p,
            "finer then coarser is the identity"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The one conversion to a float
// ---------------------------------------------------------------------------------------------------------------

/// Checks `v` against the exact value `d * 2^exp`: the error is at most half a unit in the last place, and a tie
/// went to the even mantissa.
fn assert_correctly_rounded(d: &BigInt, exp: i64, v: f64) {
    if d.is_zero() {
        assert_eq!(v, 0.0);
        return;
    }
    assert!(v.is_finite() && v != 0.0);
    assert_eq!(v < 0.0, d.is_negative());
    let bits = v.abs().to_bits();
    let biased = (bits >> 52) as i64;
    assert!(biased != 0, "the range keeps results out of the subnormals");
    let mantissa = BigInt::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52));
    let e_v = biased - 1075; // |v| = mantissa * 2^e_v
    // Bring the exact value, the result and half a unit in the last place to one common exponent.
    let s = (e_v - 1).min(exp);
    let exact = d.abs() << (exp - s) as u64;
    let got = mantissa.clone() << (e_v - s) as u64;
    let half = BigInt::from(1) << (e_v - 1 - s) as u64;
    let err = (exact - got).abs();
    assert!(
        err <= half,
        "error above half a unit in the last place: d = {d}, exp = {exp}, v = {v:e}"
    );
    if err == half {
        assert!(
            (&mantissa % BigInt::from(2)).is_zero(),
            "a tie must round to the even mantissa"
        );
    }
}

#[test]
fn float_conversion_is_correctly_rounded() {
    let mut rng = Rng(6);
    for i in 0..40_000u32 {
        let bits = rng.below(900);
        let mut d = rng.big(bits);
        // Every eighth case is an exact tie or one step off a tie, where the rounding rule is decided.
        if i % 8 == 0 && bits > 60 {
            let low = bits - 54;
            d = ((d.clone() >> low) << low) + (BigInt::from(1) << (low - 1));
            if i % 16 == 0 {
                d += BigInt::from(1);
            }
        }
        let level = Level(rng.below(21) as i32 - 10);
        let unit = Level(level.0 + rng.below(21) as i32 - 10);
        let delta = Delta {
            level,
            dx: Axis::from_big(d.clone()),
            dy: Axis::ZERO,
        };
        let exp = -54 + 10 * i64::from(unit.0 - level.0);
        match delta.to_f64(unit)[0] {
            Rel::Near(v) => {
                let magnitude_log2 = d.bits() as i64 - 1 + exp;
                if !d.is_zero() && magnitude_log2 < -1001 {
                    assert_eq!(v, 0.0, "values far below 2^-1000 units become zero");
                } else if v != 0.0 || d.is_zero() {
                    assert_correctly_rounded(&d, exp, v);
                }
            }
            Rel::Far(sign) => {
                assert!(d.bits() as i64 - 1 + exp >= 999, "only huge values are Far");
                assert_eq!(sign < 0, d.is_negative());
            }
        }
    }
}

#[test]
fn float_conversion_is_exact_for_small_values() {
    // One unit apart on the same level is exactly 1.0, at any depth.
    for level in [-1000, -1, 0, 1, 100, 1000] {
        let a = Position::origin(Level(level));
        let one_unit = Delta {
            level: Level(level),
            dx: Axis::from_i128(1 << 54),
            dy: Axis::from_i128(-(3 << 53)),
        };
        let b = a.add_delta(&one_unit);
        assert_eq!(
            b.sub(&a).to_f64(Level(level)),
            [Rel::Near(1.0), Rel::Near(-1.5)]
        );
        // The same distance measured in units of the level above is 1/1024.
        assert_eq!(
            b.sub(&a).to_f64(Level(level - 1))[0],
            Rel::Near(1.0 / 1024.0)
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn zoom_stays_in_range_and_level_changes_have_hysteresis() {
    let mut cam = Camera::new(Position::origin(Level(0)), 1.0, [1920.0, 1080.0]);
    assert_eq!(cam.level, Level(0));

    // Zoom in to just under the top of the range: still level 0.
    cam.zoom_about([960.0, 540.0], 2047.0);
    assert_eq!(cam.level, Level(0));
    // Cross the top: one level finer, and the zoom lands at 2 or more.
    cam.zoom_about([960.0, 540.0], 1.01);
    assert_eq!(cam.level, Level(1));
    assert!(cam.zoom >= 2.0 && cam.zoom < 2048.0);
    // Zooming back out a little does not change level again: it takes a factor of four.
    cam.zoom_about([960.0, 540.0], 0.5);
    assert_eq!(cam.level, Level(1));
    cam.zoom_about([960.0, 540.0], 0.24);
    assert_eq!(cam.level, Level(0));
    assert!(cam.zoom <= 512.0 && cam.zoom >= 0.5);
}

#[test]
fn a_level_change_does_not_move_anything_on_screen() {
    let mut rng = Rng(7);
    for _ in 0..500 {
        let center = deep_position(&mut rng, 40);
        let mut cam = Camera::new(center.clone(), 1500.0, [1920.0, 1080.0]);
        // A few points near the center, on the camera's level.
        let points: Vec<Position> = (0..8)
            .map(|_| {
                let d = Delta {
                    level: cam.level,
                    dx: Axis::from_i128((rng.next() as i64 >> 12) as i128),
                    dy: Axis::from_i128((rng.next() as i64 >> 12) as i128),
                };
                center.add_delta(&d)
            })
            .collect();
        let before: Vec<[f64; 2]> = points.iter().map(|p| cam.project(p).unwrap()).collect();

        // Finer: push the zoom over the top by hand and normalize. The projection must be bit-identical.
        let mut finer = cam.clone();
        finer.zoom *= 2.0;
        let scaled: Vec<[f64; 2]> = points.iter().map(|p| finer.project(p).unwrap()).collect();
        finer.normalize();
        assert_eq!(finer.level, Level(cam.level.0 + 1));
        let after: Vec<[f64; 2]> = points.iter().map(|p| finer.project(p).unwrap()).collect();
        assert_eq!(scaled, after, "going finer is lossless");

        // Coarser: the center loses its lowest ten bits, which is far below a pixel.
        cam.zoom = 0.4;
        let scaled: Vec<[f64; 2]> = points.iter().map(|p| cam.project(p).unwrap()).collect();
        cam.normalize();
        let after: Vec<[f64; 2]> = points.iter().map(|p| cam.project(p).unwrap()).collect();
        for (s, a) in scaled.iter().zip(&after) {
            assert!((s[0] - a[0]).abs() < 1e-9 && (s[1] - a[1]).abs() < 1e-9);
        }
        let _ = before;
    }
}

#[test]
fn a_point_drawn_at_depth_is_found_at_the_same_place_after_zooming_away_and_back() {
    let mut rng = Rng(8);
    for depth in [1, 10, 100, 1000] {
        let anchor = deep_position(&mut rng, depth);
        let viewport = [1920.0, 1080.0];
        let mut cam = Camera::new(anchor.clone(), 100.0, viewport);
        let start = cam.project(&anchor).unwrap();
        assert_eq!(start, [960.0, 540.0]);

        // Zoom all the way out to level 0 and back in, about the middle of the screen, a factor of 8 at a time.
        let steps = (depth as usize) * 10 / 3 + 4;
        for _ in 0..steps {
            cam.zoom_about([960.0, 540.0], 1.0 / 8.0);
        }
        assert!(
            cam.level.0 <= 1,
            "zoomed out to the top: level {}",
            cam.level.0
        );
        for _ in 0..steps {
            cam.zoom_about([960.0, 540.0], 8.0);
        }
        assert_eq!(cam.level, Level(depth));

        // The object's own position never changed, so it is exactly where it was put. The camera's center lost
        // low bits on the way out (ten per level), so the camera itself is a little off; jumping to the object
        // puts it back exactly.
        cam.set(anchor.clone(), 100.0);
        assert_eq!(cam.project(&anchor).unwrap(), [960.0, 540.0]);
        let neighbour = anchor.add_delta(&Delta {
            level: Level(depth),
            dx: Axis::from_i128(1 << 54),
            dy: Axis::ZERO,
        });
        assert_eq!(
            cam.project(&neighbour).unwrap(),
            [1060.0, 540.0],
            "one unit to the right is 100 pixels"
        );
    }
}

#[test]
fn panning_moves_the_view_by_the_pixels_asked() {
    let mut cam = Camera::new(Position::origin(Level(500)), 64.0, [800.0, 600.0]);
    let p = Position::origin(Level(500));
    assert_eq!(cam.project(&p).unwrap(), [400.0, 300.0]);
    cam.pan_by_px(128.0, -64.0);
    assert_eq!(cam.project(&p).unwrap(), [272.0, 364.0]);
    assert!(
        cam.center.x.is_small(),
        "the origin stays cheap at any level"
    );
}
