#![allow(dead_code)]

use ferrets_geometry::cell_pos::CellPos;
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    cliff_crossing::CliffCrossing,
    hierarchy::NavHierarchy,
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
    relief::Relief,
    slope::{Direction, Slope},
    terrain_id::TerrainId,
    terrain_mask::TerrainMask,
};

pub const GROUND: LayerId = LayerId::new(1);
pub const AIR: LayerId = LayerId::new(2);

/// The terrain every test grid starts as.
pub const LAND: TerrainId = TerrainId::new(1);
/// A second terrain, for cells a land shape may not enter.
pub const WATER: TerrainId = TerrainId::new(2);

pub fn world(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

pub fn nav(x: u32, y: u32) -> CellPos {
    CellPos::new(x, y)
}

/// Creates a grid of the given size with GROUND and AIR layers, every cell
/// land and free.
pub fn grid(width: u32, height: u32) -> NavGrid {
    grid_with_water(width, height, &[])
}

/// Creates a grid like [`grid`] whose `water` cells are water rather than
/// land.
pub fn grid_with_water(width: u32, height: u32, water: &[CellPos]) -> NavGrid {
    let mut grid = NavGrid::new(width, height, |cell| {
        if water.contains(&cell) { WATER } else { LAND }
    });

    grid.add_flat_layer(GROUND, 1);
    grid.add_flat_layer(AIR, 2);

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
                grid.set_occupied_by(GROUND, nav(x as u32, y as u32), true);
            }
        }
    }

    grid
}

/// The terrains a land shape may be over.
pub fn land() -> TerrainMask {
    TerrainMask::from(LAND)
}

/// Every terrain the test grids use.
pub fn every_terrain() -> TerrainMask {
    TerrainMask::from(LAND) | WATER
}

/// A single-cell shape over land on the GROUND layer — what most tests route.
pub fn on_ground() -> MoverShape {
    MoverShape::point(land(), GROUND, None)
}

/// A single-cell shape over every terrain on the AIR layer.
pub fn in_air() -> MoverShape {
    MoverShape::point(every_terrain(), AIR, None)
}

/// A `size`-by-`size` shape over land on the GROUND layer, for clearance tests.
pub fn square_on_ground(size: u32) -> MoverShape {
    MoverShape::new(land(), GROUND, GROUND, CellSize::new(size, size), None)
}

/// Builds the hierarchy for a single-cell GROUND mover only.
pub fn build_ground(grid: &NavGrid, cluster_size: u32) -> NavHierarchy {
    NavHierarchy::build(grid, cluster_size, &[on_ground()])
}

/// Builds the hierarchy for the given shapes.
pub fn build_for(grid: &NavGrid, cluster_size: u32, shapes: &[MoverShape]) -> NavHierarchy {
    NavHierarchy::build(grid, cluster_size, shapes)
}

/// The levels of [`hill_grid`], row by row: low ground at 1, a plateau at 2
/// from column 4, a mountain at 3 on its top-right corner, and a lone peak at
/// 3 at (7, 4), two levels above its west neighbor.
const HILL_LEVELS: [[u32; 8]; 5] = [
    [1, 1, 1, 1, 2, 2, 3, 3],
    [1, 1, 1, 1, 2, 2, 3, 3],
    [1, 1, 1, 1, 2, 2, 2, 2],
    [1, 1, 1, 1, 2, 2, 2, 2],
    [1, 1, 1, 1, 1, 1, 1, 3],
];

/// An 8×5 land grid with [`HILL_LEVELS`], a two-cell ramp at (3, 1) and
/// (3, 2) rising east onto the plateau, GROUND spanning levels 1 to 3 and AIR
/// at 4 over it.
pub fn hill_grid() -> NavGrid {
    let mut grid = NavGrid::new(8, 5, |_| LAND);
    grid.add_ranged_layer(GROUND, 1, 3, |cell| Relief {
        level: HILL_LEVELS[cell.y as usize][cell.x as usize],
        slope: match (cell.x, cell.y) {
            (3, 1) | (3, 2) => Some(Slope::Ramp(Direction::East)),
            _ => None,
        },
    });
    grid.add_flat_layer(AIR, 4);
    grid
}

/// A single-cell land shape on GROUND that a cliff stops.
pub fn walker() -> MoverShape {
    MoverShape::point(land(), GROUND, Some(CliffCrossing::Ramp))
}

/// A single-cell land shape on GROUND that walks up and down a cliff of one
/// level.
pub fn strider() -> MoverShape {
    MoverShape::point(land(), GROUND, Some(CliffCrossing::Cross))
}
