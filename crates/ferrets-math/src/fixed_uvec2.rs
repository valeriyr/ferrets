//! 2D unsigned fixed-point vector.

use std::ops::{Add, AddAssign, Mul, Sub};

use crate::{FixedI64, FixedU64, fixed_vec2::FixedVec2};

/// 2D unsigned vector with [`FixedU64`] components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FixedUVec2 {
    pub x: FixedU64,
    pub y: FixedU64,
}

impl FixedUVec2 {
    pub const ZERO: Self = Self::new(FixedU64::ZERO, FixedU64::ZERO);

    #[inline]
    pub const fn new(x: FixedU64, y: FixedU64) -> Self {
        Self { x, y }
    }

    /// Squared Euclidean distance to `other`.
    #[inline]
    pub fn distance_squared(self, other: Self) -> FixedU64 {
        let dx = if self.x > other.x {
            self.x - other.x
        } else {
            other.x - self.x
        };
        let dy = if self.y > other.y {
            self.y - other.y
        } else {
            other.y - self.y
        };
        dx * dx + dy * dy
    }
}

impl Add for FixedUVec2 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for FixedUVec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

// Yields FixedVec2 because the delta between two unsigned positions can be negative.
impl Sub for FixedUVec2 {
    type Output = FixedVec2;

    #[inline]
    fn sub(self, rhs: Self) -> FixedVec2 {
        FixedVec2::new(
            FixedI64::from_num(self.x) - FixedI64::from_num(rhs.x),
            FixedI64::from_num(self.y) - FixedI64::from_num(rhs.y),
        )
    }
}

impl Mul<FixedU64> for FixedUVec2 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: FixedU64) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}
