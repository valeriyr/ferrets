mod utils;

use ferrets_math::fixed_rect::FixedRect;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

/// `new` accepts pre-ordered corners and preserves them exactly.
#[test]
fn new_preserves_ordered_corners() {
    let rect = FixedRect::new(utils::vec2(1, 2), utils::vec2(5, 6));

    assert_eq!(rect.min(), utils::vec2(1, 2));
    assert_eq!(rect.max(), utils::vec2(5, 6));
}

/// A zero-size rect (single point) has zero width and height.
#[test]
fn zero_size_rect_has_zero_dimensions() {
    let rect = FixedRect::new(utils::vec2(3, 3), utils::vec2(3, 3));

    assert_eq!(rect.width(), utils::scalar(0));
    assert_eq!(rect.height(), utils::scalar(0));
}

/// `from_corners` normalizes corner order, so either corner can be the origin.
#[test]
fn from_corners_normalizes_order() {
    let a = utils::vec2(5, 8);
    let b = utils::vec2(1, 3);

    let rect = FixedRect::from_corners(a, b);

    assert_eq!(rect.min(), utils::vec2(1, 3));
    assert_eq!(rect.max(), utils::vec2(5, 8));
}

/// Width and height are derived from the normalized corners.
#[test]
fn width_and_height() {
    let rect = FixedRect::from_corners(utils::vec2(1, 2), utils::vec2(5, 6));

    assert_eq!(rect.width(), utils::scalar(4));
    assert_eq!(rect.height(), utils::scalar(4));
}

//
// ─── Contains ─────────────────────────────────────────────────────────────────
//

/// A zero-size rect contains only its single point.
#[test]
fn zero_size_rect_contains_only_its_point() {
    let rect = FixedRect::new(utils::vec2(3, 3), utils::vec2(3, 3));

    assert!(rect.contains(utils::vec2(3, 3)));
    assert!(!rect.contains(utils::vec2(4, 3)));
}

/// A point strictly inside the rect is contained.
#[test]
fn contains_interior_point() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(rect.contains(utils::vec2(5, 5)));
}

/// Points on all four boundary edges are contained.
#[test]
fn contains_boundary_points() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(rect.contains(utils::vec2(0, 5)));
    assert!(rect.contains(utils::vec2(10, 5)));
    assert!(rect.contains(utils::vec2(5, 0)));
    assert!(rect.contains(utils::vec2(5, 10)));
}

/// A point outside is not contained.
#[test]
fn does_not_contain_exterior_point() {
    let rect = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(10, 10));

    assert!(!rect.contains(utils::vec2(11, 5)));
    assert!(!rect.contains(utils::vec2(5, -1)));
}

//
// ─── Intersects ───────────────────────────────────────────────────────────────
//

/// Two overlapping rectangles intersect.
#[test]
fn overlapping_rectangles_intersect() {
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(5, 5));
    let b = FixedRect::from_corners(utils::vec2(3, 3), utils::vec2(8, 8));

    assert!(a.intersects(b));
    assert!(b.intersects(a));
}

/// Rectangles that only share an edge still intersect.
#[test]
fn touching_rectangles_intersect() {
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(5, 5));
    let b = FixedRect::from_corners(utils::vec2(5, 0), utils::vec2(10, 5));

    assert!(a.intersects(b));
}

/// Completely separate rectangles do not intersect.
#[test]
fn separate_rectangles_do_not_intersect() {
    let a = FixedRect::from_corners(utils::vec2(0, 0), utils::vec2(4, 4));
    let b = FixedRect::from_corners(utils::vec2(5, 5), utils::vec2(9, 9));

    assert!(!a.intersects(b));
}
