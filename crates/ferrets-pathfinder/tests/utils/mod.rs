#![allow(dead_code)]

use ferrets_geometry::cell_pos::CellPos;
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    hierarchy::NavHierarchy,
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
};

pub const GROUND: LayerId = LayerId::new(1);
pub const AIR: LayerId = LayerId::new(2);

pub fn world(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

pub fn nav(x: u32, y: u32) -> CellPos {
    CellPos::new(x, y)
}

/// Creates a grid of the given size with GROUND and AIR layers, all cells passable.
pub fn grid(width: u32, height: u32) -> NavGrid {
    let mut grid = NavGrid::new(width, height);

    grid.add_layer(GROUND);
    grid.add_layer(AIR);

    grid
}

/// Creates a square grid with a hollow ring of blocked border cells centered at `center`.
///
/// Only the cells at Chebyshev distance exactly `radius` from `center` are blocked;
/// the interior remains open. Cells outside the grid boundary are silently skipped.
pub fn hollow_ring_grid(grid_size: u32, center: CellPos, radius: u32) -> NavGrid {
    let mut grid = grid(grid_size, grid_size);

    let radius = radius as i32;

    for dx in -radius..=radius {
        for dy in -radius..=radius {
            if dx.abs() != radius && dy.abs() != radius {
                continue;
            }

            let x = center.x as i32 + dx;
            let y = center.y as i32 + dy;

            if x >= 0 && y >= 0 {
                grid.set_occupied(GROUND, nav(x as u32, y as u32), true);
            }
        }
    }

    grid
}

/// A single-cell shape on the GROUND layer — what most tests route.
pub fn on_ground() -> MoverShape {
    MoverShape::point(GROUND)
}

/// A single-cell shape on the AIR layer.
pub fn in_air() -> MoverShape {
    MoverShape::point(AIR)
}

/// A `size`-by-`size` shape on the GROUND layer, for clearance tests.
pub fn square_on_ground(size: u32) -> MoverShape {
    MoverShape::new(GROUND, CellSize::new(size, size))
}

/// Builds the hierarchy for a single-cell GROUND mover only.
pub fn build_ground(grid: &NavGrid, cluster_size: u32) -> NavHierarchy {
    NavHierarchy::build(grid, cluster_size, &[on_ground()])
}

/// Builds the hierarchy for the given shapes.
pub fn build_for(grid: &NavGrid, cluster_size: u32, shapes: &[MoverShape]) -> NavHierarchy {
    NavHierarchy::build(grid, cluster_size, shapes)
}
