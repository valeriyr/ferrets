//! Demo map: a 64×64 grass field with a start point in each of the four corners.
//!
//! Cells are `(x, y)` with `x` right and `y` down. Start points are indexed
//! `[top-left, bottom-right, bottom-left, top-right]`, so the first two slots sit
//! on opposite corners (a natural 1v1):
//!
//! ```text
//! y=0 +-----------------------------------------+
//! y=8 |  P0#(8,8)                    (52,8)#P3  |
//!     |     $(14,8)                 $(46,8)     |
//!     |                                         |
//! y=31|                $(31,31)                 |
//!     |                                         |
//!     |     $(14,52)               $(46,52)     |
//! y=52|  P2#(8,52)                  (52,52)#P1  |
//! y=63+-----------------------------------------+
//! ```
//!
//! `#` start/base, `$` gold mine. Each corner also has a small tree grove, and
//! four trees flank the centre mine. The camera opens framed on the local
//! player's start point.

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

/// Player start cells, one per corner, indexed by slot id.
pub const START_POINTS: [(u32, u32); 4] = [(8, 8), (52, 52), (8, 52), (52, 8)];
/// Gold mine cells: one near each start point, plus a contested one in the center.
pub const GOLD_MINES: [(u32, u32); 5] = [(14, 8), (46, 52), (14, 52), (46, 8), (31, 31)];
/// Tree cells (1×1 wood sources): a grove near each start plus a few flanking the
/// center mine.
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
    // Grove near player 2.
    (4, 47),
    (5, 47),
    (6, 47),
    (4, 48),
    (5, 48),
    (6, 48),
    // Grove near player 3.
    (57, 12),
    (58, 12),
    (59, 12),
    (57, 13),
    (58, 13),
    (59, 13),
    // Flanking the center gold mine (which sits at 31..=32, 31..=32).
    (29, 31),
    (29, 32),
    (34, 31),
    (34, 32),
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
