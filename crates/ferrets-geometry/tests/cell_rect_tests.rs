//! `CellRect` grid rectangle: construction, containment, cell enumeration,
//! and unions.

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
