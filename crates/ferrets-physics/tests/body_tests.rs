//! Bodies over the cell grid: the cell a body stands on, and the cells its
//! circle physically reaches into.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_physics::body;

//
// ─── Center cell ──────────────────────────────────────────────────────────────
//

#[test]
fn body_on_lattice_anchor_stands_on_own_cell() {
    assert_eq!(
        body::center_cell(utils::position(5.0, 5.0)),
        CellPos::new(5, 5)
    );
}

/// The claimed cell follows the visual majority of the body.
#[test]
fn body_past_half_cell_stands_on_next_cell() {
    assert_eq!(
        body::center_cell(utils::position(5.6, 5.0)),
        CellPos::new(6, 5)
    );
    assert_eq!(
        body::center_cell(utils::position(5.4, 5.0)),
        CellPos::new(5, 5)
    );
}

/// Exactly halfway rounds toward the next cell — the center sits on the
/// boundary and the boundary belongs to the cell it opens.
#[test]
fn body_at_exact_half_stands_on_next_cell() {
    assert_eq!(
        body::center_cell(utils::position(5.5, 5.0)),
        CellPos::new(6, 5)
    );
}

//
// ─── Overlapped cells ─────────────────────────────────────────────────────────
//

/// A body resting on its lattice anchor touches its neighbors' boundaries
/// exactly, and touching does not enter:
///
///   ┌───┐
///   │(○)│   one cell, four boundary touches
///   └───┘
#[test]
fn body_on_lattice_anchor_overlaps_one_cell() {
    assert_eq!(
        body::overlapped_cells(utils::position(5.0, 5.0), FixedU64::from_num(0.5)),
        vec![CellPos::new(5, 5)]
    );
}

/// A body straddling one border reaches into both cells.
#[test]
fn body_straddling_border_overlaps_both_cells() {
    assert_eq!(
        body::overlapped_cells(utils::position(5.3, 5.0), FixedU64::from_num(0.5)),
        vec![CellPos::new(5, 5), CellPos::new(6, 5)]
    );
}

/// A body near a lattice corner reaches into all four cells around it.
#[test]
fn body_near_corner_overlaps_four_cells() {
    assert_eq!(
        body::overlapped_cells(utils::position(5.3, 5.3), FixedU64::from_num(0.5)),
        vec![
            CellPos::new(5, 5),
            CellPos::new(6, 5),
            CellPos::new(5, 6),
            CellPos::new(6, 6),
        ]
    );
}

/// A wider circle covers its full bounding block when it reaches past every
/// corner.
#[test]
fn wide_body_overlaps_its_block() {
    assert_eq!(
        body::overlapped_cells(utils::position(5.0, 5.0), FixedU64::from_num(1.0)).len(),
        9
    );
}
