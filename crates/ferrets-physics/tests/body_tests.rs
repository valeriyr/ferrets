//! Bodies over the grid: the cells reach judges, lies over and reaches into.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

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
// ─── Covered rect ─────────────────────────────────────────────────────────────
//

#[test]
fn lined_up_body_covers_its_own_cells() {
    // A 2×1 anchored at (5, 5): middle (6, 5.5), bounds 5–7 by 5–6.
    assert_eq!(
        body::covered_rect(utils::location("6", "5.5", CellSize::new(2, 1))),
        CellRect::new(CellPos::new(5, 5), CellSize::new(2, 1)),
    );
}

#[test]
fn body_short_of_half_cell_covers_cell_ahead() {
    // A 1×1 with its middle at x 9.8 lies over 9.3 to 10.3: cells 9 and 10,
    // where the standing rect (floored 9 ∪ rounded 9) names 9 alone.
    assert_eq!(
        body::covered_rect(utils::location("9.8", "4.5", CellSize::ONE)),
        CellRect::new(CellPos::new(9, 4), CellSize::new(2, 1)),
    );
}

#[test]
fn body_past_corner_covers_four_cells() {
    // Middle (9.8, 5.3): 9.3 to 10.3 on x, 4.8 to 5.8 on y — cells 9–10 by 4–5.
    assert_eq!(
        body::covered_rect(utils::location("9.8", "5.3", CellSize::ONE)),
        CellRect::new(CellPos::new(9, 4), CellSize::new(2, 2)),
    );
}

#[test]
fn wide_body_across_boundary_covers_full_span() {
    // A 2×2 with its middle at x 6.6 lies over 5.6 to 7.6: cells 5, 6 and 7.
    assert_eq!(
        body::covered_rect(utils::location("6.6", "6", CellSize::new(2, 2))),
        CellRect::new(CellPos::new(5, 5), CellSize::new(3, 2)),
    );
}

//
// ─── Circle rect ──────────────────────────────────────────────────────────────
//

#[test]
fn circle_short_of_half_cell_spans_cell_ahead() {
    // A half-cell radius around x 9.8 runs from 9.3 to 10.3: cells 9 and 10 on
    // x, row 4 alone on y (4.0 to 5.0).
    assert_eq!(
        body::circle_rect(utils::point("9.8", "4.5"), utils::cells("0.5")),
        CellRect::new(CellPos::new(9, 4), CellSize::new(2, 1)),
    );
}

#[test]
fn circle_rect_spans_corner_its_circle_misses() {
    // Around (9.6, 9.6) the circle runs from 9.1 to 10.1 on each axis, so the
    // box spans 9–10 by 9–10. The far corner (10, 10) lies
    // √(0.4² + 0.4²) ≈ 0.57 from the middle, past the 0.5 radius: the circle
    // overlaps three of the box's four cells.
    let position = utils::point("9.6", "9.6");
    let radius = utils::cells("0.5");

    assert_eq!(
        body::circle_rect(position, radius),
        CellRect::new(CellPos::new(9, 9), CellSize::new(2, 2)),
    );
    assert_eq!(
        body::overlapped_cells(position, radius),
        vec![CellPos::new(9, 9), CellPos::new(10, 9), CellPos::new(9, 10)],
    );
}

#[test]
fn circle_of_no_radius_spans_cell_its_middle_lies_in() {
    // No radius around (6, 6), the corner four cells share: the box spans the
    // one cell the point lies in, (6, 6).
    assert_eq!(
        body::circle_rect(utils::point("6", "6"), FixedU64::ZERO),
        CellRect::cell(CellPos::new(6, 6)),
    );
}

//
// ─── Overlapped cells ─────────────────────────────────────────────────────────
//

/// A body lined up on its cells touches its neighbors' boundaries exactly, and
/// touching does not enter:
///
///   ┌───┐
///   │(○)│   one cell, four boundary touches
///   └───┘
#[test]
fn lined_up_body_overlaps_one_cell() {
    assert_eq!(
        body::overlapped_cells(utils::point("5.5", "5.5"), utils::cells("0.5")),
        vec![CellPos::new(5, 5)]
    );
}

/// A body straddling one border reaches into both cells.
#[test]
fn body_straddling_border_overlaps_both_cells() {
    // Around x 5.8 the circle runs from 5.3 to 6.3.
    assert_eq!(
        body::overlapped_cells(utils::point("5.8", "5.5"), utils::cells("0.5")),
        vec![CellPos::new(5, 5), CellPos::new(6, 5)]
    );
}

/// A body near a lattice corner reaches into all four cells around it.
#[test]
fn body_near_corner_overlaps_four_cells() {
    assert_eq!(
        body::overlapped_cells(utils::point("5.8", "5.8"), utils::cells("0.5")),
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
    // A radius of 1 around (5.5, 5.5) spans 4.5 to 6.5: three cells a side,
    // and each corner cell's nearest point lies √(0.5² + 0.5²) ≈ 0.71 away.
    assert_eq!(
        body::overlapped_cells(utils::point("5.5", "5.5"), FixedU64::ONE).len(),
        9
    );
}

//
// ─── Multi-cell footprints ────────────────────────────────────────────────────
//

/// The inscribed circle of a two-cell footprint lined up on its cells reaches
/// exactly its own four cells and no further:
///
///   ┌───┬───┐
///   │ ( │ ) │   four cells, boundaries touched but not crossed
///   ├───┼───┤
///   │ ( │ ) │
///   └───┴───┘
#[test]
fn multi_cell_body_overlaps_exactly_its_footprint() {
    // A 2×2 anchored at (5, 5) has its middle at 5 + 2/2 = 6.
    assert_eq!(
        body::overlapped_cells(utils::point("6", "6"), FixedU64::ONE),
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
        // At rest on (5, 5): the middle at 5 + size/2, the radius size/2.
        let half = FixedU64::from_num(size) / 2;
        let middle = FixedU64::from_num(5) + half;
        let overlapped = body::overlapped_cells(FixedUVec2::new(middle, middle), half);
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
