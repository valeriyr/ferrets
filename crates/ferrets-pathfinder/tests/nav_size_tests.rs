use ferrets_pathfinder::nav_size::NavSize;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_stores_dimensions() {
    let size = NavSize::new(3, 2);
    assert_eq!(size.width, 3);
    assert_eq!(size.height, 2);
}

#[test]
fn one_is_unit_footprint() {
    assert_eq!(NavSize::ONE, NavSize::new(1, 1));
}

#[test]
fn default_is_zero_by_zero() {
    let size = NavSize::default();
    assert_eq!(size.width, 0);
    assert_eq!(size.height, 0);
}
