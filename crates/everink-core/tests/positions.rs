//! Tests for the position type: exactness, the canonical bytes, the float conversion and the camera.

mod common;

use std::str::FromStr;

use common::{Rng, deep_position, hex};
use dashu_int::IBig;
use everink_core::wire::check_cap_bytes;
use everink_core::{Axis, Camera, CameraError, DecodeError, Delta, DepthCap, Level, Position, Rel};
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

#[test]
fn the_cap_counts_bits_not_whole_bytes() {
    let cap = DepthCap {
        max_level: 8,
        max_cell_bits: 12,
    };
    let with_cell = |cell: i64| Position {
        level: Level(0),
        x: Axis::from_cell_frac(BigInt::from(cell), 0),
        y: Axis::ZERO,
    };
    // 0x0fff is 12 bits: allowed. 0x1000 is 13 bits in the same two bytes: refused.
    assert_eq!(check_cap_bytes(&with_cell(0x0fff).encode(), &cap), Ok(()));
    assert_eq!(check_cap_bytes(&with_cell(-0x0fff).encode(), &cap), Ok(()));
    assert_eq!(
        check_cap_bytes(&with_cell(0x1000).encode(), &cap),
        Err(DecodeError::CellBeyondCap)
    );
    assert_eq!(
        Position::decode(&with_cell(-0x1000).encode(), &cap),
        Err(DecodeError::CellBeyondCap)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Exact arithmetic
// ---------------------------------------------------------------------------------------------------------------

/// The same integer in a second big-integer library, so the reference does not share code with what is tested.
fn ibig(a: &Axis) -> IBig {
    IBig::from_str(&a.to_big().to_string()).unwrap()
}

/// floor(n / 2^k) by truncating division and a correction, with no shift of a negative number.
fn floor_div_pow2(n: &IBig, k: usize) -> IBig {
    let d = IBig::from(1) << k;
    let q = n / &d;
    if *n < IBig::from(0) && &q * &d != *n {
        q - IBig::from(1)
    } else {
        q
    }
}

#[test]
fn shifts_are_exact_and_floor() {
    let mut rng = Rng(2);
    for _ in 0..20_000 {
        let a = rng.axis(400);
        let k = rng.below(300) as u32;

        // Shifting left agrees with the independent reference, whichever representation is used.
        assert_eq!(ibig(&a.shl(k)), ibig(&a) << k as usize);
        // Left then right gives the value back.
        assert_eq!(a.shl(k).shr_floor(k), a);
        // Right shift rounds toward minus infinity.
        assert_eq!(
            ibig(&a.shr_floor(k)),
            floor_div_pow2(&ibig(&a), k as usize),
            "floor failed for {a:?} >> {k}"
        );
    }
}

#[test]
fn the_inline_form_is_canonical_at_its_edges() {
    // -1 << 127 is exactly i128::MIN: it fits inline and must be held inline.
    let edge = Axis::from_i128(-1).shl(127);
    assert!(edge.is_small());
    assert_eq!(edge, Axis::from_i128(i128::MIN));
    // 1 << 127 does not fit.
    assert!(!Axis::from_i128(1).shl(127).is_small());

    // Sums and differences that cross the 128-bit edge, and come back.
    let max = Axis::from_i128(i128::MAX);
    let one = Axis::from_i128(1);
    let over = max.add(&one);
    assert!(!over.is_small());
    assert_eq!(over.sub(&one), max);
    assert!(over.sub(&one).is_small());
    let min = Axis::from_i128(i128::MIN);
    let under = min.sub(&one);
    assert!(!under.is_small());
    assert_eq!(under.add(&one), min);

    // Very large shifts of a small value floor to 0 or -1.
    for bits in [127u32, 128, 500, 100_000] {
        assert_eq!(Axis::from_i128(5).shr_floor(bits), Axis::ZERO);
        assert_eq!(Axis::from_i128(-5).shr_floor(bits), Axis::from_i128(-1));
    }
    // Equal values are equal and compare equal, whichever way they were built.
    let a = Axis::from_i128(7).shl(200).shr_floor(200);
    assert_eq!(a, Axis::from_i128(7));
    assert_eq!(a.cmp(&Axis::from_i128(7)), std::cmp::Ordering::Equal);
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

/// Difference, sum and level changes against arithmetic done in a second library from the definition:
/// a point on level `l` is the integer `n * 2^(10 * (L - l))` on any finer level `L`.
fn check_against_reference(a: &Position, b: &Position) {
    let fine = a.level.0.max(b.level.0);
    let lift = |p: &Position, axis: &Axis| ibig(axis) << (10 * (fine - p.level.0)) as usize;
    let (ax, ay) = (lift(a, &a.x), lift(a, &a.y));
    let (bx, by) = (lift(b, &b.x), lift(b, &b.y));

    let d = a.sub(b);
    assert_eq!(d.level, Level(fine));
    assert_eq!(ibig(&d.dx), &ax - &bx);
    assert_eq!(ibig(&d.dy), &ay - &by);

    let sum = b.add_delta(&d);
    assert_eq!(sum.level, Level(fine));
    assert_eq!(ibig(&sum.x), ax);
    assert_eq!(ibig(&sum.y), ay);

    let up = a.at_level(Level(fine));
    assert_eq!(ibig(&up.x), ax);
    let coarse = a.level.0.min(b.level.0);
    let down = a.at_coarser_floor(Level(coarse));
    assert_eq!(
        ibig(&down.x),
        floor_div_pow2(&ibig(&a.x), (10 * (a.level.0 - coarse)) as usize)
    );
    assert_eq!(
        ibig(&down.y),
        floor_div_pow2(&ibig(&a.y), (10 * (a.level.0 - coarse)) as usize)
    );

    assert_eq!(a.cmp_exact(b), (ax.cmp(&bx), ay.cmp(&by)));
}

#[test]
fn difference_sum_and_level_changes_agree_with_a_second_library() {
    let mut rng = Rng(4);
    // Many pairs a few levels apart, where the inline path and the big path meet.
    for _ in 0..40_000 {
        let a = rng.position(6, 140);
        let b = rng.position(6, 140);
        check_against_reference(&a, &b);
    }
    // Fewer pairs across the whole depth range: levels up to 2,048 apart.
    for _ in 0..600 {
        let a = rng.position(1024, 1_500);
        let b = rng.position(1024, 1_500);
        check_against_reference(&a, &b);
    }
    // a - a is zero.
    let a = rng.position(300, 500);
    let zero = a.sub(&a);
    assert!(zero.dx.is_zero() && zero.dy.is_zero());
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

#[test]
#[should_panic(expected = "beyond the")]
fn a_level_difference_no_surface_can_have_is_refused_not_allocated() {
    let _ = Position::origin(Level(i32::MIN)).at_level(Level(i32::MAX));
}

// ---------------------------------------------------------------------------------------------------------------
// The one conversion to a float
// ---------------------------------------------------------------------------------------------------------------

/// Checks `v` against the exact value `d * 2^exp`: the error is at most half a unit in the last place, and a tie
/// went to the even mantissa.
fn assert_correctly_rounded(d: &BigInt, exp: i64, v: f64) {
    assert!(!d.is_zero());
    assert!(
        v.is_finite() && v != 0.0,
        "a value inside the range must not become zero: d = {d}, exp = {exp}"
    );
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

fn to_f64_x(d: &BigInt, level: Level, unit: Level) -> Rel {
    Delta {
        level,
        dx: Axis::from_big(d.clone()),
        dy: Axis::ZERO,
    }
    .to_f64(unit)[0]
}

#[test]
fn float_conversion_is_correctly_rounded() {
    let mut rng = Rng(6);
    let (mut ties, mut near_ties, mut big_path) = (0u32, 0u32, 0u32);
    for i in 0..40_000u32 {
        let bits = 54 + rng.below(850);
        let mut d = rng.big(bits);
        // Every fourth case is an exact 53-bit tie, or one step off one, where the rounding rule is decided. The
        // tie is built from the value's real bit length: keep 53 bits, set the next one, clear the rest.
        if i % 4 == 0 && d.bits() > 60 {
            let low = d.bits() - 53;
            let sign = if d.is_negative() { -1 } else { 1 };
            let top = d.abs() >> low;
            d = ((top << low) + (BigInt::from(1) << (low - 1))) * sign;
            if i % 8 == 0 {
                d += BigInt::from(sign);
                near_ties += 1;
            } else {
                ties += 1;
            }
        }
        if !Axis::from_big(d.clone()).is_small() {
            big_path += 1;
        }
        let level = Level(rng.below(21) as i32 - 10);
        let unit = Level(level.0 + rng.below(21) as i32 - 10);
        let exp = -54 + 10 * i64::from(unit.0 - level.0);
        // |value| stays between about 2^-154 and 2^946 here, well inside the range: it must be Near and correct.
        match to_f64_x(&d, level, unit) {
            Rel::Near(v) => assert_correctly_rounded(&d, exp, v),
            Rel::Far(_) => panic!("a value inside the range came back Far: d = {d}, exp = {exp}"),
        }
    }
    assert!(
        ties > 4_000 && near_ties > 4_000,
        "the tie cases were generated: {ties} and {near_ties}"
    );
    assert!(
        big_path > 20_000,
        "the big-integer path was exercised: {big_path}"
    );
}

#[test]
fn float_conversion_handles_the_ends_of_its_range() {
    let l = Level(0);
    let one = BigInt::from(1);
    // With unit level equal to the level, a step is 2^-54 units, so d = 2^1054 is exactly 2^1000 units.
    let top = one.clone() << 1054u32;
    assert_eq!(to_f64_x(&top, l, l), Rel::Far(1), "2^1000 units is Far");
    assert_eq!(to_f64_x(&(-top.clone()), l, l), Rel::Far(-1));
    // One step under 2^1000 rounds up to 2^1000, and Far is decided on the rounded value.
    assert_eq!(to_f64_x(&(&top - &one), l, l), Rel::Far(1));
    // The largest value that rounds below 2^1000: 53 one-bits just under it.
    let just_under = &top - (one.clone() << 1001u32);
    match to_f64_x(&just_under, l, l) {
        Rel::Near(v) => {
            assert!(v < f64::from_bits((2023u64) << 52) && v > f64::from_bits((2022u64) << 52));
            assert_correctly_rounded(&just_under, -54, v);
        }
        Rel::Far(_) => panic!("a value under 2^1000 must be Near"),
    }

    // Small end. Measured in units of a level 95 coarser, a step is 2^(-54 - 950) = 2^-1004 units.
    let coarse = Level(-95);
    let two_pow_minus_1000 = f64::from_bits((23u64) << 52); // exponent -1000
    assert_eq!(
        to_f64_x(&BigInt::from(16), l, coarse),
        Rel::Near(two_pow_minus_1000),
        "2^-1000 is kept"
    );
    assert_eq!(
        to_f64_x(&BigInt::from(-16), l, coarse),
        Rel::Near(-two_pow_minus_1000)
    );
    assert_eq!(
        to_f64_x(&BigInt::from(15), l, coarse),
        Rel::Near(0.0),
        "just under 2^-1000 becomes zero"
    );
    assert_eq!(to_f64_x(&BigInt::from(1), l, coarse), Rel::Near(0.0));
    assert_eq!(to_f64_x(&BigInt::zero(), l, coarse), Rel::Near(0.0));
    // A value that rounds up across 2^-1000 from below is kept: 2^53 + ... is not needed; the rule is on the
    // rounded value, as at the top.
    let rounds_up = (one.clone() << 60u32) - &one; // 60 one-bits: rounds to 2^60
    assert_eq!(
        to_f64_x(&rounds_up, l, Level(-100)),
        Rel::Near(f64::from_bits((1023u64 + 60 - 54 - 1000) << 52))
    );
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

const VIEW: [f64; 2] = [1920.0, 1080.0];
const MID: [f64; 2] = [960.0, 540.0];

fn offset(p: &Position, dx: i128, dy: i128) -> Position {
    p.add_delta(&Delta {
        level: p.level,
        dx: Axis::from_i128(dx),
        dy: Axis::from_i128(dy),
    })
}

#[test]
fn zoom_stays_in_range_and_level_changes_have_hysteresis() {
    let mut cam = Camera::new(Position::origin(Level(0)), 1.0, VIEW).unwrap();
    assert_eq!(cam.level, Level(0));

    // Zoom in to just under the top of the range: still level 0.
    cam.zoom_about(MID, 1023.0).unwrap();
    cam.zoom_about(MID, 2.0).unwrap();
    assert_eq!(cam.level, Level(0));
    assert!(cam.zoom < 2048.0);
    // Cross the top: one level finer, and the zoom lands at 2 or more.
    cam.zoom_about(MID, 1.01).unwrap();
    assert_eq!(cam.level, Level(1));
    assert!(cam.zoom >= 2.0 && cam.zoom < 2048.0);
    // Zooming back out a little does not change level again: it takes a factor of four.
    cam.zoom_about(MID, 0.5).unwrap();
    assert_eq!(cam.level, Level(1));
    cam.zoom_about(MID, 0.24).unwrap();
    assert_eq!(cam.level, Level(0));
    assert!(cam.zoom <= 512.0 && cam.zoom >= 0.5);
}

#[test]
fn a_level_change_does_not_move_anything_on_screen() {
    let mut rng = Rng(7);
    for _ in 0..500 {
        let center = deep_position(&mut rng, 40);
        // Points near the center, on the camera's level.
        let points: Vec<Position> = (0..8)
            .map(|_| {
                offset(
                    &center,
                    (rng.next() as i64 >> 12) as i128,
                    (rng.next() as i64 >> 12) as i128,
                )
            })
            .collect();
        let project_all = |cam: &Camera| -> Vec<[f64; 2]> {
            points.iter().map(|p| cam.project(p).unwrap()).collect()
        };

        // Finer: a zoom just under the top of the range, then a factor of two takes it across. The center is the
        // same point exactly, the zoom is exactly twice the old one divided by 1024, and nothing on screen moves
        // by more than float rounding in the projection.
        let mut cam = Camera::new(center.clone(), 1500.0, VIEW).unwrap();
        let before = project_all(&cam);
        cam.zoom_about(MID, 2.0).unwrap();
        assert_eq!(
            cam.level,
            Level(41),
            "the zoom crossed into the finer level"
        );
        assert!(
            cam.center.same_point(&center),
            "going finer keeps the center exactly"
        );
        assert_eq!(cam.zoom, 1500.0 * 2.0 / 1024.0, "the zoom change is exact");
        let after = project_all(&cam);
        for (b, a) in before.iter().zip(&after) {
            assert!(((b[0] - 960.0) * 2.0 + 960.0 - a[0]).abs() < 1e-9);
            assert!(((b[1] - 540.0) * 2.0 + 540.0 - a[1]).abs() < 1e-9);
        }

        // Coarser: the center is floored to the coarser level's steps, so it moves by less than 2^10 fine steps
        // (under 2^-44 of a unit), which is far below a pixel.
        let mut cam = Camera::new(center.clone(), 0.6, VIEW).unwrap();
        let before = project_all(&cam);
        cam.zoom_about(MID, 0.5).unwrap();
        assert_eq!(
            cam.level,
            Level(39),
            "the zoom crossed into the coarser level"
        );
        assert_eq!(cam.zoom, 0.6 * 0.5 * 1024.0);
        let moved = center.sub(&cam.center);
        for d in [&moved.dx, &moved.dy] {
            assert!(
                *d >= Axis::ZERO && *d < Axis::from_i128(1 << 10),
                "the center floored by less than 2^10 steps"
            );
        }
        let after = project_all(&cam);
        for (b, a) in before.iter().zip(&after) {
            assert!(((b[0] - 960.0) * 0.5 + 960.0 - a[0]).abs() < 1e-9);
            assert!(((b[1] - 540.0) * 0.5 + 540.0 - a[1]).abs() < 1e-9);
        }
    }
}

#[test]
fn one_zoom_step_keeps_the_point_under_the_focus() {
    let mut rng = Rng(8);
    for depth in [0, 1, 10, 100, 1000] {
        for _ in 0..300 {
            let center = deep_position(&mut rng, depth);
            let zoom = 0.6 + (rng.below(2_000_000) as f64) / 1000.0; // 0.6 to about 2000
            let mut cam = Camera::new(center.clone(), zoom, VIEW).unwrap();
            // A point somewhere on screen, away from the middle, and the pixel it is under.
            let px = [rng.below(1920) as f64 + 0.25, rng.below(1080) as f64 + 0.75];
            let steps = |p: f64, mid: f64| {
                ((p - mid) / cam.zoom * (1u64 << 54) as f64).round_ties_even() as i128
            };
            let point = offset(
                &cam.center.clone(),
                steps(px[0], 960.0),
                steps(px[1], 540.0),
            );
            let focus = cam.project(&point).unwrap();
            assert!((focus[0] - px[0]).abs() < 1e-6 && (focus[1] - px[1]).abs() < 1e-6);

            let factor = [0.125, 0.5, 0.9, 1.1, 2.0, 8.0, 700.0][rng.below(7) as usize];
            cam.zoom_about(focus, factor).unwrap();
            let after = cam.project(&point).unwrap();
            assert!(
                (after[0] - focus[0]).abs() < 1e-6 && (after[1] - focus[1]).abs() < 1e-6,
                "depth {depth}, zoom {zoom}, factor {factor}: {focus:?} moved to {after:?}"
            );
        }
    }
}

#[test]
fn zooming_out_and_back_while_following_a_point_keeps_it_in_place() {
    // A person who zooms out and back in while keeping the pointer on a thing: the focus of each step is where the
    // thing is on screen now. The error per step is tiny and does not grow with the zoom.
    let mut rng = Rng(9);
    for depth in [1, 10, 100] {
        let anchor = deep_position(&mut rng, depth);
        let start = [700.25, 300.75];
        let mut cam = Camera::new(anchor.clone(), 100.0, VIEW).unwrap();
        cam.pan_by_px(960.0 - start[0], 540.0 - start[1]).unwrap();
        let first = cam.project(&anchor).unwrap();
        assert!((first[0] - start[0]).abs() < 1e-6 && (first[1] - start[1]).abs() < 1e-6);

        let steps = (depth as usize) * 10 / 3 + 4;
        for _ in 0..steps {
            let focus = cam.project(&anchor).unwrap();
            cam.zoom_about(focus, 1.0 / 8.0).unwrap();
        }
        assert!(
            cam.level.0 <= 1,
            "zoomed out to the top: level {}",
            cam.level.0
        );
        for _ in 0..steps {
            let focus = cam.project(&anchor).unwrap();
            cam.zoom_about(focus, 8.0).unwrap();
        }
        assert_eq!(cam.level, Level(depth));
        let end = cam.project(&anchor).unwrap();
        assert!(
            (end[0] - start[0]).abs() < 1e-3 && (end[1] - start[1]).abs() < 1e-3,
            "depth {depth}: started at {start:?}, ended at {end:?}"
        );
    }
}

#[test]
fn an_object_never_moves_and_a_jump_lands_on_it_exactly() {
    // The camera's center loses ten bits per level on the way out, and zooming in about a fixed screen point
    // multiplies whatever error there is. So a round trip about the middle of the screen does not return to the
    // same spot at depth, and is not meant to. What holds is that the object's own position is untouched, and that
    // a jump (`set`) lands on it exactly, whatever the camera did before.
    let mut rng = Rng(10);
    for depth in [1, 10, 100, 1000] {
        let anchor = deep_position(&mut rng, depth);
        let stored = anchor.encode();
        let mut cam = Camera::new(anchor.clone(), 100.0, VIEW).unwrap();
        assert_eq!(cam.project(&anchor).unwrap(), MID);

        let steps = (depth as usize) * 10 / 3 + 4;
        for _ in 0..steps {
            cam.zoom_about(MID, 1.0 / 8.0).unwrap();
        }
        for _ in 0..steps {
            cam.zoom_about(MID, 8.0).unwrap();
        }
        assert_eq!(cam.level, Level(depth));
        assert_eq!(
            anchor.encode(),
            stored,
            "the object's position is untouched by the camera"
        );

        cam.set(anchor.clone(), 100.0).unwrap();
        assert_eq!(cam.project(&anchor).unwrap(), MID);
        let neighbour = offset(&anchor, 1 << 54, 0);
        assert_eq!(
            cam.project(&neighbour).unwrap(),
            [1060.0, 540.0],
            "one unit to the right is 100 pixels"
        );
    }
}

#[test]
fn panning_moves_the_view_by_the_pixels_asked() {
    let mut cam = Camera::new(Position::origin(Level(500)), 64.0, [800.0, 600.0]).unwrap();
    let p = Position::origin(Level(500));
    assert_eq!(cam.project(&p).unwrap(), [400.0, 300.0]);
    cam.pan_by_px(128.0, -64.0).unwrap();
    assert_eq!(cam.project(&p).unwrap(), [272.0, 364.0]);
    assert!(
        cam.center.x.is_small(),
        "the origin stays cheap at any level"
    );
}

#[test]
fn the_camera_refuses_numbers_that_are_not_gestures() {
    let origin = Position::origin(Level(3));
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            Camera::new(origin.clone(), bad, VIEW).is_err(),
            "zoom {bad}"
        );
    }
    assert_eq!(
        Camera::new(origin.clone(), 1.0, [f64::NAN, 1.0]).err(),
        Some(CameraError::NotFinite)
    );

    let mut cam = Camera::new(origin.clone(), 10.0, VIEW).unwrap();
    let before = (cam.level, cam.center.clone(), cam.zoom);
    let unchanged = |cam: &Camera| (cam.level, cam.center.clone(), cam.zoom) == before;

    assert_eq!(cam.pan_by_px(f64::NAN, 0.0), Err(CameraError::NotFinite));
    assert_eq!(
        cam.pan_by_px(0.0, f64::NEG_INFINITY),
        Err(CameraError::NotFinite)
    );
    assert_eq!(cam.pan_by_px(1e300, 0.0), Err(CameraError::OutOfRange));
    // The second axis is bad: the first must not have been applied.
    assert_eq!(cam.pan_by_px(5.0, f64::NAN), Err(CameraError::NotFinite));
    assert!(unchanged(&cam));

    assert_eq!(cam.zoom_about(MID, f64::NAN), Err(CameraError::NotFinite));
    assert_eq!(
        cam.zoom_about(MID, f64::INFINITY),
        Err(CameraError::NotFinite)
    );
    assert_eq!(cam.zoom_about(MID, 0.0), Err(CameraError::OutOfRange));
    assert_eq!(cam.zoom_about(MID, -2.0), Err(CameraError::OutOfRange));
    assert_eq!(cam.zoom_about(MID, 1e9), Err(CameraError::OutOfRange));
    assert_eq!(
        cam.zoom_about([f64::NAN, 0.0], 2.0),
        Err(CameraError::NotFinite)
    );
    assert_eq!(
        cam.set(origin.clone(), f64::NAN),
        Err(CameraError::NotFinite)
    );
    assert_eq!(cam.set(origin, 0.0), Err(CameraError::OutOfRange));
    assert!(unchanged(&cam));

    // The largest allowed values work.
    cam.pan_by_px(1e12, -1e12).unwrap();
    cam.zoom_about(MID, 1024.0).unwrap();
    cam.zoom_about(MID, 1.0 / 1024.0).unwrap();
}
