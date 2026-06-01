//! 2D signed fixed-point vector.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use crate::FixedI64;

/// 2D vector with [`FixedI64`] components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FixedVec2 {
    pub x: FixedI64,
    pub y: FixedI64,
}

impl FixedVec2 {
    pub const ZERO: Self = Self {
        x: FixedI64::ZERO,
        y: FixedI64::ZERO,
    };

    #[inline]
    pub fn new(x: FixedI64, y: FixedI64) -> Self {
        Self { x, y }
    }

    /// Squared Euclidean distance to `other`.
    #[inline]
    pub fn distance_squared(self, other: Self) -> FixedI64 {
        let d = self - other;
        d.x * d.x + d.y * d.y
    }

    /// Dot product. Positive when vectors point in the same general direction,
    /// zero when perpendicular, negative when opposing.
    #[inline]
    pub fn dot(self, other: Self) -> FixedI64 {
        self.x * other.x + self.y * other.y
    }
}

impl Add for FixedVec2 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for FixedVec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for FixedVec2 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for FixedVec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul<FixedI64> for FixedVec2 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: FixedI64) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Neg for FixedVec2 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}
