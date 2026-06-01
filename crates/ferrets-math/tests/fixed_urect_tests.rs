mod utils;

use ferrets_math::fixed_urect::FixedURect;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

/// `new` accepts pre-ordered corners and preserves them exactly.
#[test]
fn new_preserves_ordered_corners() {
    let rect = FixedURect::new(utils::uvec2(1, 2), utils::uvec2(5, 6));

    assert_eq!(rect.min(), utils::uvec2(1, 2));
    assert_eq!(rect.max(), utils::uvec2(5, 6));
}

/// A zero-size rect (single point) has zero width and height.
#[test]
fn zero_size_rect_has_zero_dimensions() {
    let rect = FixedURect::new(utils::uvec2(3, 3), utils::uvec2(3, 3));

    assert_eq!(rect.width(), utils::uscalar(0));
    assert_eq!(rect.height(), utils::uscalar(0));
}

/// `from_corners` normalizes corner order, so either corner can be the origin.
#[test]
fn from_corners_normalizes_order() {
    let a = utils::uvec2(5, 8);
    let b = utils::uvec2(1, 3);

    let rect = FixedURect::from_corners(a, b);

    assert_eq!(rect.min(), utils::uvec2(1, 3));
    assert_eq!(rect.max(), utils::uvec2(5, 8));
}

/// Width and height are derived from the normalized corners.
#[test]
fn width_and_height() {
    let rect = FixedURect::from_corners(utils::uvec2(1, 2), utils::uvec2(5, 6));

    assert_eq!(rect.width(), utils::uscalar(4));
    assert_eq!(rect.height(), utils::uscalar(4));
}

//
// ─── Contains ─────────────────────────────────────────────────────────────────
//

/// A zero-size rect contains only its single point.
#[test]
fn zero_size_rect_contains_only_its_point() {
    let rect = FixedURect::new(utils::uvec2(3, 3), utils::uvec2(3, 3));

    assert!(rect.contains(utils::uvec2(3, 3)));
    assert!(!rect.contains(utils::uvec2(4, 3)));
}

/// A point strictly inside the rect is contained.
#[test]
fn contains_interior_point() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(rect.contains(utils::uvec2(5, 5)));
}

/// Points on all four boundary edges are contained.
#[test]
fn contains_boundary_points() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(rect.contains(utils::uvec2(0, 5)));
    assert!(rect.contains(utils::uvec2(10, 5)));
    assert!(rect.contains(utils::uvec2(5, 0)));
    assert!(rect.contains(utils::uvec2(5, 10)));
}

/// A point outside is not contained.
#[test]
fn does_not_contain_exterior_point() {
    let rect = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(10, 10));

    assert!(!rect.contains(utils::uvec2(11, 5)));
    assert!(!rect.contains(utils::uvec2(5, 11)));
}

//
// ─── Intersects ───────────────────────────────────────────────────────────────
//

/// Two overlapping rectangles intersect.
#[test]
fn overlapping_rectangles_intersect() {
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(5, 5));
    let b = FixedURect::from_corners(utils::uvec2(3, 3), utils::uvec2(8, 8));

    assert!(a.intersects(b));
    assert!(b.intersects(a));
}

/// Rectangles that only share an edge still intersect.
#[test]
fn touching_rectangles_intersect() {
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(5, 5));
    let b = FixedURect::from_corners(utils::uvec2(5, 0), utils::uvec2(10, 5));

    assert!(a.intersects(b));
}

/// Completely separate rectangles do not intersect.
#[test]
fn separate_rectangles_do_not_intersect() {
    let a = FixedURect::from_corners(utils::uvec2(0, 0), utils::uvec2(4, 4));
    let b = FixedURect::from_corners(utils::uvec2(5, 5), utils::uvec2(9, 9));

    assert!(!a.intersects(b));
}
