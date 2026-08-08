mod utils;

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

#[test]
fn arithmetic_results_are_bit_identical() {
    let a = utils::uvec2(3, 4);
    let b = utils::uvec2(1, 2);

    let r1 = a + b;
    let r2 = a + b;

    assert_eq!(r1.x.to_bits(), r2.x.to_bits());
    assert_eq!(r1.y.to_bits(), r2.y.to_bits());
}

//
// ─── Arithmetic ───────────────────────────────────────────────────────────────
//

#[test]
fn adding_zero_is_identity() {
    let v = utils::uvec2(7, 3);

    assert_eq!(v + FixedUVec2::ZERO, v);
}

#[test]
fn scalar_multiplication() {
    assert_eq!(utils::uvec2(3, 2) * utils::uscalar(4), utils::uvec2(12, 8));
}

//
// ─── Compound assignment ──────────────────────────────────────────────────────
//

#[test]
fn add_assign() {
    let mut v = utils::uvec2(3, 4);

    v += utils::uvec2(1, 1);
    assert_eq!(v, utils::uvec2(4, 5));
}

//
// ─── Subtraction ──────────────────────────────────────────────────────────────
//

#[test]
fn subtraction_yields_positive_direction() {
    let a = utils::uvec2(5, 8);
    let b = utils::uvec2(2, 3);

    assert_eq!(a - b, utils::vec2(3, 5));
}

#[test]
fn subtraction_yields_negative_direction() {
    let a = utils::uvec2(2, 3);
    let b = utils::uvec2(5, 8);

    assert_eq!(a - b, utils::vec2(-3, -5));
}

//
// ─── Length ───────────────────────────────────────────────────────────────────
//

/// A 3-4-5 right triangle: the hypotenuse of a Pythagorean triple is exact.
#[test]
fn length_of_pythagorean_triple_is_exact() {
    assert_eq!(utils::uvec2(3, 4).length(), utils::uscalar(5));
}

/// Fractional components resolve exactly when the triple scales cleanly:
/// (1.5, 2) is the 3-4-5 triangle halved.
#[test]
fn length_of_fractional_triple_is_exact() {
    let v = FixedUVec2::new(FixedU64::from_num(1.5), FixedU64::from_num(2));

    assert_eq!(v.length(), FixedU64::from_num(2.5));
}

#[test]
fn length_of_zero_vector_is_zero() {
    assert_eq!(FixedUVec2::ZERO.length(), FixedU64::ZERO);
}

#[test]
fn length_along_single_axis_keeps_component() {
    let v = FixedUVec2::new(FixedU64::from_num(7.25), FixedU64::ZERO);

    assert_eq!(v.length(), FixedU64::from_num(7.25));
}

/// An irrational length rounds down as tightly as the precision allows:
/// length² ≤ 2 < (length + one bit)², squared in exact integer space — the
/// fixed multiply truncates and would hide the tightness this asserts.
#[test]
fn irrational_length_rounds_down_within_precision() {
    let length = utils::uvec2(1, 1).length();

    let squared = |bits: u128| bits * bits;
    let two = (FixedU64::from_num(2).to_bits() as u128) << 32;
    assert!(squared(length.to_bits() as u128) <= two);
    assert!(squared(length.to_bits() as u128 + 1) > two);
}

/// The largest single-axis vector is the boundary: its length is itself, one
/// bit short of overflowing.
#[test]
fn length_of_maximal_single_axis_vector_fits() {
    let v = FixedUVec2::new(FixedU64::MAX, FixedU64::ZERO);

    assert_eq!(v.length(), FixedU64::MAX);
}

#[test]
#[should_panic(expected = "offsets this long do not fit the coordinate space")]
fn length_beyond_coordinate_space_panics() {
    FixedUVec2::new(FixedU64::MAX, FixedU64::MAX).length();
}

//
// ─── Distance ─────────────────────────────────────────────────────────────────
//

#[test]
fn distance_is_length_of_offset() {
    assert_eq!(
        utils::uvec2(2, 3).distance(utils::uvec2(5, 7)),
        utils::uscalar(5)
    );
}

#[test]
fn distance_is_symmetric() {
    let a = utils::uvec2(10, 5);
    let b = utils::uvec2(3, 1);

    assert_eq!(a.distance(b), b.distance(a));
}

/// Each axis can independently be larger or smaller — both orderings are handled.
#[test]
fn distance_handles_mixed_axis_ordering() {
    let a = utils::uvec2(1, 8);
    let b = utils::uvec2(4, 4);

    assert_eq!(a.distance(b), utils::uscalar(5));
    assert_eq!(b.distance(a), utils::uscalar(5));
}

#[test]
fn distance_to_self_is_zero() {
    let v = utils::uvec2(6, 9);

    assert_eq!(v.distance(v), FixedU64::ZERO);
}
