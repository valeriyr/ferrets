//! 2D unsigned fixed-point vector.

use std::ops::{Add, AddAssign, Mul, Sub};

use serde::{Deserialize, Serialize};

use crate::{FixedI64, FixedU64, fixed_vec2::FixedVec2};

/// 2D unsigned vector with [`FixedU64`] components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
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

    /// Euclidean length, rounded down to the type's precision.
    ///
    /// Exact to the last bit: with 32 fractional bits,
    /// `isqrt(x_bits² + y_bits²)` *is* the result's raw representation, so
    /// no precision is shed squaring or summing.
    ///
    /// # Panics
    ///
    /// Panics when the squared length overflows the widened intermediate —
    /// possible only for a vector longer than the coordinate space is wide.
    pub fn length(self) -> FixedU64 {
        let x = self.x.to_bits() as u128;
        let y = self.y.to_bits() as u128;
        // Each square fits: bits ≤ 2⁶⁴ − 1, so bits² < 2¹²⁸. Only the sum
        // can overflow — and any sum that fits roots back into range, since
        // isqrt of a `u128` always fits a `u64`.
        let sum = (x * x)
            .checked_add(y * y)
            .expect("offsets this long do not fit the coordinate space");
        FixedU64::from_bits(sum.isqrt() as u64)
    }

    /// Euclidean distance to `other`, rounded down to the type's precision —
    /// the [`length`](Self::length) of the offset between the two points.
    ///
    /// # Panics
    ///
    /// Panics when the distance overflows [`FixedU64`] (see
    /// [`length`](Self::length)).
    pub fn distance(self, other: Self) -> FixedU64 {
        Self::new(self.x.abs_diff(other.x), self.y.abs_diff(other.y)).length()
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
