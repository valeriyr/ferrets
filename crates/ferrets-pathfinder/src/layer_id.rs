//! A navigation grid layer identifier.

use std::{
    fmt::Display,
    ops::{BitAnd, BitOr, Deref},
};

use crate::layer_mask::LayerMask;

/// Identifies a single navigation layer.
///
/// Each value is a non-zero power of two. Use `|` to union layers into a [`LayerMask`],
/// or `&` to intersect them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(u32);

impl LayerId {
    /// Creates a `LayerId` from a raw bit value.
    ///
    /// Panics if `value` is zero or not a power of two.
    pub const fn new(value: u32) -> Self {
        assert!(
            value != 0 && value.is_power_of_two(),
            "LayerId must be a non-zero power of two"
        );
        Self(value)
    }
}

impl From<u32> for LayerId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl Display for LayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for LayerId {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl BitAnd for LayerId {
    type Output = LayerMask;

    fn bitand(self, rhs: Self) -> LayerMask {
        LayerMask::from(self.0 & rhs.0)
    }
}

impl BitOr for LayerId {
    type Output = LayerMask;

    fn bitor(self, rhs: Self) -> LayerMask {
        LayerMask::from(self.0 | rhs.0)
    }
}
