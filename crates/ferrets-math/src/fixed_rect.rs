//! Axis-aligned fixed-point rectangle.

use crate::FixedI64;

use crate::fixed_vec2::FixedVec2;

/// Axis-aligned bounding rectangle with [`FixedI64`] coordinates.
///
/// Used for selection boxes, building footprints, and overlap checks.
///
/// The invariant `min.x <= max.x` and `min.y <= max.y` is always upheld.
/// Use [`FixedRect::new`] when corners are already ordered, or
/// [`FixedRect::from_corners`] to normalize arbitrary corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedRect {
    min: FixedVec2,
    max: FixedVec2,
}

impl FixedRect {
    /// Creates a rect from pre-ordered `min` and `max` corners.
    ///
    /// Panics in debug builds if `min.x > max.x` or `min.y > max.y`.
    #[inline]
    pub fn new(min: FixedVec2, max: FixedVec2) -> Self {
        debug_assert!(min.x <= max.x && min.y <= max.y);
        Self { min, max }
    }

    /// Creates a rect from any two opposite corners, normalizing the order.
    #[inline]
    pub fn from_corners(a: FixedVec2, b: FixedVec2) -> Self {
        Self {
            min: FixedVec2::new(a.x.min(b.x), a.y.min(b.y)),
            max: FixedVec2::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    #[inline]
    pub fn min(self) -> FixedVec2 {
        self.min
    }

    #[inline]
    pub fn max(self) -> FixedVec2 {
        self.max
    }

    #[inline]
    pub fn width(self) -> FixedI64 {
        self.max.x - self.min.x
    }

    #[inline]
    pub fn height(self) -> FixedI64 {
        self.max.y - self.min.y
    }

    /// Returns `true` if `point` lies inside or on the boundary.
    #[inline]
    pub fn contains(self, point: FixedVec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Returns `true` if the two rects overlap (touching edges count).
    #[inline]
    pub fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}
