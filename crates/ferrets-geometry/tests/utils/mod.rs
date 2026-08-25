#![allow(dead_code)]

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

pub fn nav(x: u32, y: u32) -> CellPos {
    CellPos::new(x, y)
}

pub fn world(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// A world position part way across its cells, written as decimal digits rather
/// than a float, so the position is the one the digits name.
pub fn part_way(x: &str, y: &str) -> FixedUVec2 {
    let coordinate = |text: &str| {
        FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a world coordinate"))
    };
    FixedUVec2::new(coordinate(x), coordinate(y))
}
