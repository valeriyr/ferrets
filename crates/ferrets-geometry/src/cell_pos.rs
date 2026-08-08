//! Grid cell coordinate.

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::cell_rect::CellRect;

/// One grid cell corresponds to one world unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CellPos {
    pub x: u32,
    pub y: u32,
}

impl CellPos {
    #[inline]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    /// Returns the cell of `rect` nearest to `self`, clamping each axis into
    /// the rectangle's span. The result is independent of the projection.
    ///
    /// `rect` must cover at least one cell — an empty rectangle has no
    /// nearest cell to name.
    #[inline]
    pub fn clamp_to_rect(self, rect: CellRect) -> CellPos {
        debug_assert!(
            rect.size.width > 0 && rect.size.height > 0,
            "an empty rectangle has no nearest cell"
        );
        CellPos::new(
            self.x
                .clamp(rect.origin.x, rect.origin.x + rect.size.width - 1),
            self.y
                .clamp(rect.origin.y, rect.origin.y + rect.size.height - 1),
        )
    }
}

/// Converts a world position to the grid cell it occupies.
impl From<FixedUVec2> for CellPos {
    #[inline]
    fn from(pos: FixedUVec2) -> Self {
        Self {
            x: pos.x.floor().to_num::<u32>(),
            y: pos.y.floor().to_num::<u32>(),
        }
    }
}

/// World position of the grid cell's origin corner.
impl From<CellPos> for FixedUVec2 {
    #[inline]
    fn from(pos: CellPos) -> Self {
        FixedUVec2::new(FixedU64::from_num(pos.x), FixedU64::from_num(pos.y))
    }
}
