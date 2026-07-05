mod utils;

use ferrets_math::fixed_rect::FixedRect;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_preserves_ordered_corners() {
    let rect = FixedRect::new(utils::vec2(1, 2), utils::vec2(5, 6));

    assert_eq!(rect.min(), utils::vec2(1, 2));
    assert_eq!(rect.max(), utils::vec2(5, 6));
}

#[test]
fn zero_size_rect_has_zero_dimensions() {
    let rect = FixedRect::new(utils::vec2(3, 3), utils::vec2(3, 3));

    assert_eq!(rect.width(), utils::scalar(0));
    assert_eq!(rect.height(), utils::scalar(0));
}

#[test]
fn from_corners_normalizes_order() {
    let a = utils::vec2(5, 8);
    let b = utils::vec2(1, 3);

    let rect = FixedRect::from_corners(a, b);

    assert_eq!(rect.min(), utils::vec2(1, 3));
    assert_eq!(rect.max(), utils::vec2(5, 8));
}

#[test]
fn width_and_height() {
    let rect = FixedRect::from_corners(utils::vec2(1, 2), utils::vec2(5, 6));

    assert_eq!(rect.width(), utils::scalar(4));
    assert_eq!(rect.height(), utils::scalar(4));
}

//
// ─── Contains ─────────────────────────────────────────────────────────────────
//

#[test]
fn zero_size_rect_contains_only_its_point() {
    let rect = FixedRect::new(utils::vec2(3, 3), utils::vec2(3, 3));

    assert!(rect.contains(utils::vec2(3, 3)));
    assert!(!rect.contains(utils::vec2(4, 3)));
}

#[test]
fn contains_interior_point() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(rect.contains(utils::vec2(5, 5)));
}

#[test]
fn contains_boundary_points() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(rect.contains(utils::vec2(0, 5)));
    assert!(rect.contains(utils::vec2(10, 5)));
    assert!(rect.contains(utils::vec2(5, 0)));
    assert!(rect.contains(utils::vec2(5, 10)));
}

#[test]
fn does_not_contain_exterior_point() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(!rect.contains(utils::vec2(11, 5)));
    assert!(!rect.contains(utils::vec2(5, -1)));
}

//
// ─── Intersects ───────────────────────────────────────────────────────────────
//

#[test]
fn overlapping_rectangles_intersect() {
    // +-------+
    // | a     |
    // |   +---+---+
    // +---|---+   |
    //     |   b   |
    //     +-------+
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(5, 5));
    let b = FixedRect::from_corners(utils::vec2(3, 3), utils::vec2(8, 8));

    assert!(a.intersects(b));
    assert!(b.intersects(a));
}

#[test]
fn touching_rectangles_intersect() {
    // +-------+-------+
    // |   a   |   b   |
    // +-------+-------+
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(5, 5));
    let b = FixedRect::from_corners(utils::vec2(5, 0), utils::vec2(10, 5));

    assert!(a.intersects(b));
}

#[test]
fn separate_rectangles_do_not_intersect() {
    // +-------+
    // |   a   |
    // +-------+
    //              +-------+
    //              |   b   |
    //              +-------+
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(4, 4));
    let b = FixedRect::from_corners(utils::vec2(5, 5), utils::vec2(9, 9));

    assert!(!a.intersects(b));
}

//
// ─── Serialization ──────────────────────────────────────────────────────────
//

#[test]
fn round_trips_through_bcs() {
    let rect = FixedRect::from_corners(utils::vec2(-3, -2), utils::vec2(4, 6));

    let bytes = bcs::to_bytes(&rect).expect("encode");
    let decoded: FixedRect = bcs::from_bytes(&bytes).expect("decode");

    assert_eq!(decoded, rect);
}

// Decoding routes through `new`, whose debug-assert rejects `min > max`. A struct
// `{ min, max }` and the tuple `(min, max)` encode identically under bcs, so this
// crafts bytes for a rect whose invariant is violated.
//
// Debug-only: the assert is compiled out of release builds.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "min.x <= max.x")]
fn deserializing_min_greater_than_max_panics() {
    let bad = bcs::to_bytes(&(utils::vec2(5, 5), utils::vec2(-1, -1))).expect("encode");
    let _: FixedRect = bcs::from_bytes(&bad).expect("decode");
}
