//! Axis-aligned unsigned fixed-point rectangle.

use crate::{FixedU64, fixed_uvec2::FixedUVec2};

/// Axis-aligned bounding rectangle with [`FixedU64`] coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedURect {
    min: FixedUVec2,
    max: FixedUVec2,
}

impl FixedURect {
    /// Creates a rect from pre-ordered `min` and `max` corners.
    ///
    /// Panics in debug builds if `min.x > max.x` or `min.y > max.y`.
    #[inline]
    pub fn new(min: FixedUVec2, max: FixedUVec2) -> Self {
        debug_assert!(min.x <= max.x && min.y <= max.y);
        Self { min, max }
    }

    /// Creates a rect from any two opposite corners, normalizing the order.
    #[inline]
    pub fn from_corners(a: FixedUVec2, b: FixedUVec2) -> Self {
        Self {
            min: FixedUVec2::new(a.x.min(b.x), a.y.min(b.y)),
            max: FixedUVec2::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    #[inline]
    pub fn min(self) -> FixedUVec2 {
        self.min
    }

    #[inline]
    pub fn max(self) -> FixedUVec2 {
        self.max
    }

    #[inline]
    pub fn width(self) -> FixedU64 {
        self.max.x - self.min.x
    }

    #[inline]
    pub fn height(self) -> FixedU64 {
        self.max.y - self.min.y
    }

    /// Returns `true` if `point` lies inside or on the boundary.
    #[inline]
    pub fn contains(self, point: FixedUVec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Returns `true` if the two rectangles overlap (touching edges count).
    #[inline]
    pub fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}
