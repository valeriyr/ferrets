mod utils;

use ferrets_math::fixed_uvec2::FixedUVec2;

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
// ─── Geometry ─────────────────────────────────────────────────────────────────
//

/// A 3-4-5 right triangle has squared hypotenuse length 25.
#[test]
fn distance_squared_three_four_five() {
    assert_eq!(
        FixedUVec2::ZERO.distance_squared(utils::uvec2(3, 4)),
        utils::uscalar(25)
    );
}

#[test]
fn distance_squared_is_symmetric() {
    let a = utils::uvec2(10, 5);
    let b = utils::uvec2(3, 1);

    assert_eq!(a.distance_squared(b), b.distance_squared(a));
}

/// Each axis can independently be larger or smaller — both orderings are handled.
#[test]
fn distance_squared_mixed_axis_ordering() {
    let a = utils::uvec2(1, 8);
    let b = utils::uvec2(4, 4);

    assert_eq!(a.distance_squared(b), utils::uscalar(25));
    assert_eq!(b.distance_squared(a), utils::uscalar(25));
}
