#![allow(dead_code)]

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

pub fn nav(x: u32, y: u32) -> CellPos {
    CellPos::new(x, y)
}

pub fn world(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}
