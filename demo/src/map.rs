//! Demo map: a 64×64 grass field with two player start points.

use ferrets_pathfinder::{
    astar::Projection,
    nav_grid::{LayerId, NavGrid},
    nav_pos::NavPos,
};
use ferrets_simulation::map::Map;

/// The single ground navigation layer used by the demo.
pub const GROUND: LayerId = LayerId::new(1);
pub const WIDTH: u32 = 64;
pub const HEIGHT: u32 = 64;

/// Player start cells, indexed by slot id (0 = local, 1 = enemy).
pub const START_POINTS: [(u32, u32); 2] = [(8, 8), (52, 52)];
/// A gold mine cell near each start point.
pub const GOLD_MINES: [(u32, u32); 2] = [(14, 8), (46, 52)];
/// Tree cells (1×1 wood sources): a grove near each start plus a central belt.
pub const TREES: &[(u32, u32)] = &[
    // Grove near player 0.
    (4, 12),
    (5, 12),
    (6, 12),
    (4, 13),
    (5, 13),
    (6, 13),
    // Grove near player 1.
    (57, 50),
    (58, 50),
    (59, 50),
    (57, 51),
    (58, 51),
    (59, 51),
    // Central belt.
    (30, 31),
    (31, 31),
    (32, 31),
    (33, 31),
    (31, 32),
    (32, 32),
];

/// Builds the demo map.
pub fn build() -> Map {
    let mut nav_grid = NavGrid::new(WIDTH, HEIGHT);
    nav_grid.add_layer(GROUND);

    let start_points = START_POINTS
        .iter()
        .map(|&(x, y)| NavPos::new(x, y))
        .collect();

    Map::new("demo", Projection::Isometric, nav_grid, start_points)
}
