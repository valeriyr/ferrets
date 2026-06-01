mod utils;

use ferrets_math::fixed_vec2::FixedVec2;

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

/// The same sequence of operations must produce bit-identical results every
/// time — the core guarantee required for lockstep multiplayer and replays.
#[test]
fn arithmetic_results_are_bit_identical() {
    let a = utils::vec2(3, 4);
    let b = utils::vec2(1, 2);

    let r1 = a + b - b * utils::scalar(2);
    let r2 = a + b - b * utils::scalar(2);

    assert_eq!(r1.x.to_bits(), r2.x.to_bits());
    assert_eq!(r1.y.to_bits(), r2.y.to_bits());
}

//
// ─── Arithmetic ───────────────────────────────────────────────────────────────
//

/// Zero is the additive identity: adding it changes nothing.
#[test]
fn adding_zero_is_identity() {
    let v = utils::vec2(7, -3);

    assert_eq!(v + FixedVec2::ZERO, v);
}

/// Scalar multiplication scales both components uniformly.
#[test]
fn scalar_multiplication() {
    assert_eq!(utils::vec2(3, -2) * utils::scalar(4), utils::vec2(12, -8));
}

/// Negating a vector and adding it back yields zero.
#[test]
fn adding_negation_yields_zero() {
    let v = utils::vec2(5, -2);

    assert_eq!(v + (-v), FixedVec2::ZERO);
}

//
// ─── Compound assignment ──────────────────────────────────────────────────────
//

/// `+=` and `-=` modify in place identically to their non-assigning forms.
#[test]
fn add_assign_and_sub_assign() {
    let mut v = utils::vec2(3, 4);

    v += utils::vec2(1, 1);
    assert_eq!(v, utils::vec2(4, 5));

    v -= utils::vec2(2, 3);
    assert_eq!(v, utils::vec2(2, 2));
}

//
// ─── Geometry ─────────────────────────────────────────────────────────────────
//

/// A 3-4-5 right triangle has squared hypotenuse length 25.
#[test]
fn distance_squared_three_four_five() {
    assert_eq!(
        FixedVec2::ZERO.distance_squared(utils::vec2(3, 4)),
        utils::scalar(25)
    );
}

/// Dot product is positive for same-direction vectors, zero for perpendicular.
#[test]
fn dot_product() {
    let right = utils::vec2(1, 0);
    let up = utils::vec2(0, 1);

    assert_eq!(right.dot(right), utils::scalar(1));
    assert_eq!(right.dot(up), utils::scalar(0));
    assert_eq!(right.dot(-right), utils::scalar(-1));
}

/// Distance is symmetric: the direction of measurement does not matter.
#[test]
fn distance_squared_is_symmetric() {
    let a = utils::vec2(10, 5);
    let b = utils::vec2(3, 1);

    assert_eq!(a.distance_squared(b), b.distance_squared(a));
}
