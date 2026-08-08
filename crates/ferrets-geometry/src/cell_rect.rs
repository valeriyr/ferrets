//! Grid-aligned rectangle of cells.

use crate::{cell_pos::CellPos, cell_size::CellSize};

/// A rectangle of whole cells: an origin and the footprint it spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRect {
    /// The rectangle's lowest-coordinate cell.
    pub origin: CellPos,
    /// How many cells the rectangle spans on each axis.
    pub size: CellSize,
}

impl CellRect {
    #[inline]
    pub const fn new(origin: CellPos, size: CellSize) -> Self {
        Self { origin, size }
    }

    /// The rectangle covering a single cell.
    #[inline]
    pub const fn cell(origin: CellPos) -> Self {
        Self::new(origin, CellSize::ONE)
    }

    /// Returns `true` if the rectangle covers `pos`.
    #[inline]
    pub fn contains(self, pos: CellPos) -> bool {
        pos.x >= self.origin.x
            && pos.x < self.origin.x + self.size.width
            && pos.y >= self.origin.y
            && pos.y < self.origin.y + self.size.height
    }

    /// The cells the rectangle covers, in row-major order.
    pub fn cells(self) -> impl Iterator<Item = CellPos> {
        (0..self.size.height).flat_map(move |dy| {
            (0..self.size.width).map(move |dx| CellPos::new(self.origin.x + dx, self.origin.y + dy))
        })
    }

    /// The smallest rectangle covering both `self` and `other`.
    pub fn union(self, other: CellRect) -> CellRect {
        let origin = CellPos::new(
            self.origin.x.min(other.origin.x),
            self.origin.y.min(other.origin.y),
        );
        let end_x = (self.origin.x + self.size.width).max(other.origin.x + other.size.width);
        let end_y = (self.origin.y + self.size.height).max(other.origin.y + other.size.height);
        CellRect::new(origin, CellSize::new(end_x - origin.x, end_y - origin.y))
    }
}
