//! Grid lines: the cells and corners a segment between cell centers crosses.

use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    grid_line::{self, Crossing},
};

//
// ─── Crossings ────────────────────────────────────────────────────────────────
//

#[test]
fn straight_line_crosses_each_cell_of_its_row() {
    assert_eq!(
        grid_line::crossings(one(0, 0), one(3, 0)).collect::<Vec<_>>(),
        vec![cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0)]
    );
}

#[test]
fn diagonal_passes_through_corners_between_its_cells() {
    assert_eq!(
        grid_line::crossings(one(0, 0), one(2, 2)).collect::<Vec<_>>(),
        vec![
            cell(0, 0),
            corner(1, 1),
            cell(1, 1),
            corner(2, 2),
            cell(2, 2)
        ]
    );
}

#[test]
fn oblique_line_crosses_cells_in_order() {
    // From (0.5, 0.5) to (2.5, 1.5): x = 1 at a quarter of the way (y 0.75),
    // y = 1 at half (x 1.5), x = 2 at three quarters (y 1.25).
    assert_eq!(
        grid_line::crossings(one(0, 0), one(2, 1)).collect::<Vec<_>>(),
        vec![cell(0, 0), cell(1, 0), cell(1, 1), cell(2, 1)]
    );
}

#[test]
fn line_running_backward_crosses_cells_from_its_start() {
    assert_eq!(
        grid_line::crossings(one(2, 1), one(0, 0)).collect::<Vec<_>>(),
        vec![cell(2, 1), cell(1, 1), cell(1, 0), cell(0, 0)]
    );
}

#[test]
fn line_from_square_center_leaves_its_start_corner_out() {
    assert_eq!(
        grid_line::crossings(square(0, 0), one(2, 2)).collect::<Vec<_>>(),
        vec![cell(1, 1), corner(2, 2), cell(2, 2)]
    );
}

#[test]
fn line_along_cell_edge_crosses_only_corners() {
    assert_eq!(
        // From the corner (1, 1) to the corner (3, 1) along the edge between
        // rows 0 and 1, passing the corner (2, 1) between them.
        grid_line::crossings(square(0, 0), square(2, 0)).collect::<Vec<_>>(),
        vec![corner(2, 1)]
    );
}

#[test]
fn line_from_cell_to_itself_crosses_that_cell() {
    assert_eq!(
        grid_line::crossings(one(1, 1), one(1, 1)).collect::<Vec<_>>(),
        vec![cell(1, 1)]
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn one(x: u32, y: u32) -> CellRect {
    CellRect::cell(CellPos::new(x, y))
}

fn square(x: u32, y: u32) -> CellRect {
    CellRect::new(CellPos::new(x, y), CellSize::new(2, 2))
}

fn cell(x: u32, y: u32) -> Crossing {
    Crossing::Cell(CellPos::new(x, y))
}

fn corner(x: u32, y: u32) -> Crossing {
    Crossing::Corner(CellPos::new(x, y))
}
