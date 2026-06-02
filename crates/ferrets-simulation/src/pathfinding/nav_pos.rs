//! Navigation grid coordinate.

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

/// One grid cell corresponds to one world unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NavPos {
    pub x: u32,
    pub y: u32,
}

impl NavPos {
    #[inline]
    pub fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }
}

/// Converts a world position to the grid cell it occupies.
impl From<FixedUVec2> for NavPos {
    #[inline]
    fn from(pos: FixedUVec2) -> Self {
        Self {
            x: pos.x.floor().to_num::<u32>(),
            y: pos.y.floor().to_num::<u32>(),
        }
    }
}

/// World position of the grid cell's origin corner.
impl From<NavPos> for FixedUVec2 {
    #[inline]
    fn from(pos: NavPos) -> Self {
        FixedUVec2::new(FixedU64::from_num(pos.x), FixedU64::from_num(pos.y))
    }
}
