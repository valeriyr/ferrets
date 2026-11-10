//! Grid cell coordinate.

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use serde::{Deserialize, Serialize};

use crate::cell_rect::CellRect;

/// One grid cell corresponds to one world unit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct CellPos {
    pub x: u32,
    pub y: u32,
}

impl CellPos {
    #[inline]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    /// The middle of the cell.
    #[inline]
    pub fn middle(self) -> FixedUVec2 {
        let half = FixedU64::ONE / 2;
        FixedUVec2::new(
            FixedU64::from_num(self.x) + half,
            FixedU64::from_num(self.y) + half,
        )
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

/// The point at the cell's corner.
impl From<CellPos> for FixedUVec2 {
    #[inline]
    fn from(pos: CellPos) -> Self {
        FixedUVec2::new(FixedU64::from_num(pos.x), FixedU64::from_num(pos.y))
    }
}
