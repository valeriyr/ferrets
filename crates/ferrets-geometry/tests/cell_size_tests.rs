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

//
// ─── Fit ──────────────────────────────────────────────────────────────────────
//

#[test]
fn size_fits_within_area_no_wider_and_no_taller() {
    let area = CellSize::new(32, 16);

    assert!(CellSize::new(32, 16).fits_within(area));
    assert!(CellSize::new(2, 2).fits_within(area));
    assert!(!CellSize::new(33, 1).fits_within(area));
    assert!(!CellSize::new(1, 17).fits_within(area));
}
