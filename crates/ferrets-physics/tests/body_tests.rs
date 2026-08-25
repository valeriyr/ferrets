//! Bodies over the cell grid: the cell a body stands on, the cells a reach
//! measure judges it from, and the cells its circle physically reaches into.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_physics::body;

//
// ─── Anchor ───────────────────────────────────────────────────────────────────
//

#[test]
fn body_on_lattice_anchor_stands_on_own_cell() {
    assert_eq!(body::anchor(utils::position("5", "5")), CellPos::new(5, 5));
}

/// The claimed cell follows the visual majority of the body.
#[test]
fn body_past_half_cell_stands_on_next_cell() {
    assert_eq!(
        body::anchor(utils::position("5.6", "5")),
        CellPos::new(6, 5)
    );
    assert_eq!(
        body::anchor(utils::position("5.4", "5")),
        CellPos::new(5, 5)
    );
}

/// Exactly halfway rounds toward the next cell — the center sits on the
/// boundary and the boundary belongs to the cell it opens.
#[test]
fn body_at_exact_half_stands_on_next_cell() {
    assert_eq!(
        body::anchor(utils::position("5.5", "5")),
        CellPos::new(6, 5)
    );
}

//
// ─── Standing rect ────────────────────────────────────────────────────────────
//

/// A body on its lattice point floors and rounds to the same cell, so the cells
/// it stands on are exactly its footprint.
#[test]
fn body_on_lattice_stands_on_own_footprint() {
    assert_eq!(
        body::standing_rect(utils::position("5", "5"), CellSize::ONE),
        CellRect::new(CellPos::new(5, 5), CellSize::ONE),
    );
    assert_eq!(
        body::standing_rect(utils::position("5", "5"), CellSize::new(3, 3)),
        CellRect::new(CellPos::new(5, 5), CellSize::new(3, 3)),
    );
}

/// A body lying across a boundary stands on the cells either side of it: the
/// one its position floors into and the one its center rounds to.
#[test]
fn body_across_boundary_stands_on_both_cells() {
    assert_eq!(
        body::standing_rect(utils::position("5.6", "5"), CellSize::ONE),
        CellRect::new(CellPos::new(5, 5), CellSize::new(2, 1)),
    );
}

/// The diagonal case a single quantization cannot express: a body past a corner
/// floors one cell short on one axis and rounds one cell long on the other, and
/// the cells it stands on cover both readings.
#[test]
fn body_past_corner_stands_on_cells_of_both_readings() {
    assert_eq!(
        body::standing_rect(utils::position("5.6", "5.7"), CellSize::ONE),
        CellRect::new(CellPos::new(5, 5), CellSize::new(2, 2)),
    );
}

/// A wide body's cells extend from each reading, so its far edge grows with it.
#[test]
fn wide_body_across_boundary_stands_on_cells_of_full_span() {
    assert_eq!(
        body::standing_rect(utils::position("5.6", "5"), CellSize::new(2, 2)),
        CellRect::new(CellPos::new(5, 5), CellSize::new(3, 2)),
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
        body::overlapped_cells(
            utils::position("5", "5"),
            CellSize::ONE,
            FixedU64::from_num(0.5)
        ),
        vec![CellPos::new(5, 5)]
    );
}

/// A body straddling one border reaches into both cells.
#[test]
fn body_straddling_border_overlaps_both_cells() {
    assert_eq!(
        body::overlapped_cells(
            utils::position("5.3", "5"),
            CellSize::ONE,
            FixedU64::from_num(0.5)
        ),
        vec![CellPos::new(5, 5), CellPos::new(6, 5)]
    );
}

/// A body near a lattice corner reaches into all four cells around it.
#[test]
fn body_near_corner_overlaps_four_cells() {
    assert_eq!(
        body::overlapped_cells(
            utils::position("5.3", "5.3"),
            CellSize::ONE,
            FixedU64::from_num(0.5)
        ),
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
        body::overlapped_cells(
            utils::position("5", "5"),
            CellSize::ONE,
            FixedU64::from_num(1.0)
        )
        .len(),
        9
    );
}

//
// ─── Multi-cell footprints ────────────────────────────────────────────────────
//

/// A footprint's circle sits half a footprint past its anchor, so a two-cell
/// body is centered on the lattice corner its four cells meet at — not half a
/// cell in, which would hang it off its own footprint's near side.
#[test]
fn multi_cell_body_is_centered_on_its_footprint() {
    assert_eq!(
        body::center(utils::position("5", "5"), CellSize::new(2, 2)),
        utils::position("6", "6")
    );
    assert_eq!(
        body::center(utils::position("5", "5"), CellSize::ONE),
        utils::position("5.5", "5.5")
    );
}

/// The inscribed circle of a two-cell footprint resting on its anchor reaches
/// exactly its own four cells and no further:
///
///   ┌───┬───┐
///   │ ( │ ) │   four cells, boundaries touched but not crossed
///   ├───┼───┤
///   │ ( │ ) │
///   └───┴───┘
#[test]
fn multi_cell_body_overlaps_exactly_its_footprint() {
    assert_eq!(
        body::overlapped_cells(
            utils::position("5", "5"),
            CellSize::new(2, 2),
            FixedU64::from_num(1.0)
        ),
        vec![
            CellPos::new(5, 5),
            CellPos::new(6, 5),
            CellPos::new(5, 6),
            CellPos::new(6, 6),
        ]
    );
}

/// A body's circle grows with its footprint, so a wider body still reaches
/// only its own cells: the inscribed radius is what keeps a footprint's
/// physical presence inside the cells the planner cleared for it.
#[test]
fn inscribed_circle_never_leaves_its_footprint() {
    for size in [1, 2, 3] {
        let footprint = CellSize::new(size, size);
        let overlapped = body::overlapped_cells(
            utils::position("5", "5"),
            footprint,
            FixedU64::from_num(size) / 2,
        );
        assert_eq!(
            overlapped.len() as u32,
            size * size,
            "a {size}-cell body reached outside its own footprint"
        );
        assert!(
            overlapped
                .iter()
                .all(|cell| (5..5 + size).contains(&cell.x) && (5..5 + size).contains(&cell.y)),
            "a {size}-cell body's cells are not its footprint's: {overlapped:?}"
        );
    }
}
