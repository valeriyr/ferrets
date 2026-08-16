//! `CellRect` grid rectangle: construction, containment, cell enumeration,
//! unions, and low-side growth.

mod utils;

use ferrets_geometry::{cell_rect::CellRect, cell_size::CellSize};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_stores_origin_and_size() {
    let rect = CellRect::new(utils::nav(3, 4), CellSize::new(2, 5));

    assert_eq!(rect.origin, utils::nav(3, 4));
    assert_eq!(rect.size, CellSize::new(2, 5));
}

#[test]
fn cell_covers_single_cell() {
    assert_eq!(
        CellRect::cell(utils::nav(7, 2)),
        CellRect::new(utils::nav(7, 2), CellSize::ONE)
    );
}

//
// ─── contains ─────────────────────────────────────────────────────────────────
//
// A 2×2 rectangle at origin (3,3):
//
// . . . . . .   y=2
// . . . R R o   y=3   R = rectangle cells (3,3) (4,3) (3,4) (4,4)
// . . . R R .   y=4   o = (5,3), the first cell past the right edge
// . . . o . .   y=5   o = (3,5), the first cell past the bottom edge
//

#[test]
fn contains_every_covered_cell() {
    let rect = CellRect::new(utils::nav(3, 3), CellSize::new(2, 2));

    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        assert!(rect.contains(utils::nav(cell.0, cell.1)), "cell {cell:?}");
    }
}

#[test]
fn contains_excludes_cells_past_edges() {
    let rect = CellRect::new(utils::nav(3, 3), CellSize::new(2, 2));

    assert!(!rect.contains(utils::nav(5, 3)));
    assert!(!rect.contains(utils::nav(3, 5)));
    assert!(!rect.contains(utils::nav(2, 3)));
    assert!(!rect.contains(utils::nav(3, 2)));
}

//
// ─── cells ────────────────────────────────────────────────────────────────────
//

#[test]
fn cells_enumerate_in_row_major_order() {
    let rect = CellRect::new(utils::nav(3, 3), CellSize::new(2, 2));

    assert_eq!(
        rect.cells().collect::<Vec<_>>(),
        vec![
            utils::nav(3, 3),
            utils::nav(4, 3),
            utils::nav(3, 4),
            utils::nav(4, 4)
        ]
    );
}

#[test]
fn single_cell_rect_enumerates_its_origin() {
    assert_eq!(
        CellRect::cell(utils::nav(6, 1)).cells().collect::<Vec<_>>(),
        vec![utils::nav(6, 1)]
    );
}

//
// ─── union ────────────────────────────────────────────────────────────────────
//
// Two disjoint rectangles and the union that covers them and the gap between:
//
// A A . . . .   y=0   A = (0,0) 2×2
// A A . . . .   y=1   B = (4,2) 2×1
// . . . . B B   y=2   union = (0,0) 6×3
//

#[test]
fn union_covers_both_rects_and_gap() {
    let a = CellRect::new(utils::nav(0, 0), CellSize::new(2, 2));
    let b = CellRect::new(utils::nav(4, 2), CellSize::new(2, 1));

    assert_eq!(
        a.union(b),
        CellRect::new(utils::nav(0, 0), CellSize::new(6, 3))
    );
    assert_eq!(a.union(b), b.union(a), "union is symmetric");
}

#[test]
fn union_of_nested_rects_is_outer_rect() {
    let outer = CellRect::new(utils::nav(1, 1), CellSize::new(4, 4));
    let inner = CellRect::new(utils::nav(2, 2), CellSize::new(2, 2));

    assert_eq!(outer.union(inner), outer);
}

#[test]
fn union_with_self_is_identity() {
    let rect = CellRect::new(utils::nav(3, 5), CellSize::new(2, 3));

    assert_eq!(rect.union(rect), rect);
}

//
// ─── grown_low ────────────────────────────────────────────────────────────────
//
// A 1×1 goal at (10,5) grown for a 2×2 footprint:
//
// . E E   y=4   E = the growth, up-left of the goal only
// . E G   y=5   G = the original cell — the far corner never moves
//   ↑
//  x=9
//

#[test]
fn grown_low_grows_toward_low_coordinates_only() {
    let goal = CellRect::cell(utils::nav(10, 5));

    assert_eq!(
        goal.grown_low(CellSize::new(2, 2)),
        CellRect::new(utils::nav(9, 4), CellSize::new(2, 2))
    );
}

#[test]
fn grown_low_keeps_far_edge_when_clamped_at_grid_origin() {
    // The origin cannot go below zero, and the growth must not leak to the
    // far sides instead: the end stays exactly where it was.
    let goal = CellRect::new(utils::nav(1, 0), CellSize::new(2, 2));

    assert_eq!(
        goal.grown_low(CellSize::new(3, 3)),
        CellRect::new(utils::nav(0, 0), CellSize::new(3, 2))
    );
}

#[test]
fn grown_low_is_identity_for_single_cell_footprint() {
    let goal = CellRect::new(utils::nav(4, 4), CellSize::new(2, 3));

    assert_eq!(goal.grown_low(CellSize::ONE), goal);
}

#[test]
#[should_panic(expected = "size dimensions must be greater than 0")]
fn grown_low_panics_on_zero_size() {
    CellRect::cell(utils::nav(4, 4)).grown_low(CellSize::new(0, 1));
}

//
// ─── accepted_by ──────────────────────────────────────────────────────────────
//

#[test]
fn accepted_by_grows_low_for_ranged_stop() {
    // The goal grows low by the footprint, so an anchor's distance to it
    // equals the footprint's nearest-edge distance to the original goal.
    let goal = CellRect::cell(utils::nav(10, 5));

    assert_eq!(
        goal.accepted_by(CellSize::new(2, 2), 2),
        CellRect::new(utils::nav(9, 4), CellSize::new(2, 2))
    );
}

#[test]
fn accepted_by_keeps_goal_for_zero_stop() {
    // Standing on the goal footprint means the anchor itself does, so the
    // rect stays as ordered even for a wide footprint.
    let goal = CellRect::cell(utils::nav(10, 5));

    assert_eq!(goal.accepted_by(CellSize::new(2, 2), 0), goal);
}

#[test]
fn accepted_by_is_identity_for_single_cell_footprint_at_any_stop() {
    let goal = CellRect::new(utils::nav(4, 4), CellSize::new(2, 2));

    for stop in 0..3 {
        assert_eq!(goal.accepted_by(CellSize::ONE, stop), goal);
    }
}

//
// ─── intersects ───────────────────────────────────────────────────────────────
//

#[test]
fn rects_sharing_cells_intersect() {
    let a = CellRect::new(utils::nav(2, 2), CellSize::new(2, 2));
    let b = CellRect::new(utils::nav(3, 3), CellSize::new(2, 2));

    assert!(a.intersects(b));
    assert!(b.intersects(a));
}

#[test]
fn rect_contains_smaller_rect_intersects() {
    let outer = CellRect::new(utils::nav(1, 1), CellSize::new(4, 4));
    let inner = CellRect::new(utils::nav(2, 2), CellSize::ONE);

    assert!(outer.intersects(inner));
    assert!(inner.intersects(outer));
}

#[test]
fn edge_adjacent_rects_do_not_intersect() {
    // Touching along an edge shares no cell: cells are whole units, and the
    // rect ending at x 3 leaves x 4 to its neighbor.
    let a = CellRect::new(utils::nav(2, 2), CellSize::new(2, 2));
    let b = CellRect::new(utils::nav(4, 2), CellSize::new(2, 2));

    assert!(!a.intersects(b));
    assert!(!b.intersects(a));
}
