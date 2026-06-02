mod utils;

use ferrets_math::fixed_urect::FixedURect;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_preserves_ordered_corners() {
    let rect = FixedURect::new(utils::uvec2(1, 2), utils::uvec2(5, 6));

    assert_eq!(rect.min(), utils::uvec2(1, 2));
    assert_eq!(rect.max(), utils::uvec2(5, 6));
}

#[test]
fn zero_size_rect_has_zero_dimensions() {
    let rect = FixedURect::new(utils::uvec2(3, 3), utils::uvec2(3, 3));

    assert_eq!(rect.width(), utils::uscalar(0));
    assert_eq!(rect.height(), utils::uscalar(0));
}

#[test]
fn from_corners_normalizes_order() {
    let a = utils::uvec2(5, 8);
    let b = utils::uvec2(1, 3);

    let rect = FixedURect::from_corners(a, b);

    assert_eq!(rect.min(), utils::uvec2(1, 3));
    assert_eq!(rect.max(), utils::uvec2(5, 8));
}

#[test]
fn width_and_height() {
    let rect = FixedURect::from_corners(utils::uvec2(1, 2), utils::uvec2(5, 6));

    assert_eq!(rect.width(), utils::uscalar(4));
    assert_eq!(rect.height(), utils::uscalar(4));
}

//
// ─── Contains ─────────────────────────────────────────────────────────────────
//

#[test]
fn zero_size_rect_contains_only_its_point() {
    let rect = FixedURect::new(utils::uvec2(3, 3), utils::uvec2(3, 3));

    assert!(rect.contains(utils::uvec2(3, 3)));
    assert!(!rect.contains(utils::uvec2(4, 3)));
}

#[test]
fn contains_interior_point() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(rect.contains(utils::uvec2(5, 5)));
}

#[test]
fn contains_boundary_points() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(rect.contains(utils::uvec2(0, 5)));
    assert!(rect.contains(utils::uvec2(10, 5)));
    assert!(rect.contains(utils::uvec2(5, 0)));
    assert!(rect.contains(utils::uvec2(5, 10)));
}

#[test]
fn does_not_contain_exterior_point() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(!rect.contains(utils::uvec2(11, 5)));
    assert!(!rect.contains(utils::uvec2(5, 11)));
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
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(5, 5));
    let b = FixedURect::from_corners(utils::uvec2(3, 3), utils::uvec2(8, 8));

    assert!(a.intersects(b));
    assert!(b.intersects(a));
}

#[test]
fn touching_rectangles_intersect() {
    // +-------+-------+
    // |   a   |   b   |
    // +-------+-------+
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(5, 5));
    let b = FixedURect::from_corners(utils::uvec2(5, 0), utils::uvec2(10, 5));

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
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(4, 4));
    let b = FixedURect::from_corners(utils::uvec2(5, 5), utils::uvec2(9, 9));

    assert!(!a.intersects(b));
}
