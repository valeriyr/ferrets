#![allow(dead_code)]

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::pathfinding::{
    nav_grid::{LayerId, NavGrid},
    nav_pos::NavPos,
};

pub const GROUND: LayerId = 1;
pub const AIR: LayerId = 2;

pub fn world(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

pub fn nav(x: u32, y: u32) -> NavPos {
    NavPos::new(x, y)
}

pub fn grid(width: u32, height: u32) -> NavGrid {
    let mut grid = NavGrid::new(width, height);

    grid.add_layer(GROUND);
    grid.add_layer(AIR);

    grid
}
