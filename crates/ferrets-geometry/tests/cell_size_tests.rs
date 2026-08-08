use ferrets_geometry::cell_size::CellSize;

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_stores_dimensions() {
    let size = CellSize::new(3, 2);

    assert_eq!(size.width, 3);
    assert_eq!(size.height, 2);
}

#[test]
fn one_is_unit_footprint() {
    assert_eq!(CellSize::ONE, CellSize::new(1, 1));
}

#[test]
fn default_is_zero_by_zero() {
    let size = CellSize::default();

    assert_eq!(size.width, 0);
    assert_eq!(size.height, 0);
}
