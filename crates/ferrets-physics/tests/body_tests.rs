//! Bodies over the grid: the cells reach judges.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_vec2::FixedVec2;
use ferrets_physics::body::{self, Heading};

//
// ─── Standing rect ────────────────────────────────────────────────────────────
//

/// A body lined up on its cells floors and rounds to the same cell, so the
/// cells it stands on are exactly its footprint.
#[test]
fn lined_up_body_stands_on_own_footprint() {
    // A 1×1 anchored at (5, 5) has its middle at 5 + 1/2; a 3×3 at 5 + 3/2.
    assert_eq!(
        body::standing_rect(utils::location("5.5", "5.5", CellSize::ONE)),
        CellRect::new(CellPos::new(5, 5), CellSize::ONE),
    );
    assert_eq!(
        body::standing_rect(utils::location("6.5", "6.5", CellSize::new(3, 3))),
        CellRect::new(CellPos::new(5, 5), CellSize::new(3, 3)),
    );
}

/// A body lying across a boundary stands on the cells either side of it: the
/// one its low edge floors into and the one it is anchored at.
#[test]
fn body_across_boundary_stands_on_both_cells() {
    // Middle 6.1, low edge 5.6: floored 5, rounded 6.
    assert_eq!(
        body::standing_rect(utils::location("6.1", "5.5", CellSize::ONE)),
        CellRect::new(CellPos::new(5, 5), CellSize::new(2, 1)),
    );
}

/// The diagonal case a single quantization cannot express: a body past a corner
/// floors one cell short on one axis and rounds one cell long on the other, and
/// the cells it stands on cover both readings.
#[test]
fn body_past_corner_stands_on_cells_of_both_readings() {
    // Low edge (5.6, 5.7): floored (5, 5), rounded (6, 6).
    assert_eq!(
        body::standing_rect(utils::location("6.1", "6.2", CellSize::ONE)),
        CellRect::new(CellPos::new(5, 5), CellSize::new(2, 2)),
    );
}

/// A wide body's cells extend from each reading, so its far edge grows with it.
#[test]
fn wide_body_across_boundary_stands_on_cells_of_full_span() {
    // A 2×2 with its middle at x 6.6: low edge 5.6, floored 5, rounded 6, so
    // columns 5 to 6 + 2 − 1 = 7.
    assert_eq!(
        body::standing_rect(utils::location("6.6", "6", CellSize::new(2, 2))),
        CellRect::new(CellPos::new(5, 5), CellSize::new(3, 2)),
    );
}

//
// ─── Heading ──────────────────────────────────────────────────────────────────
//

/// A walk aiming at the point it stands on aims nowhere.
#[test]
fn heading_toward_no_offset_is_unaimed() {
    assert_eq!(Heading::toward(FixedVec2::ZERO), Heading::Unaimed);
    assert_eq!(
        Heading::toward(utils::push("1", "0")),
        Heading::Toward(utils::push("1", "0"))
    );
}
