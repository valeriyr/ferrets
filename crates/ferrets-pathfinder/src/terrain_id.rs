//! A terrain identifier.

use std::{fmt::Display, ops::Deref};

/// Identifies a single terrain.
///
/// Each value is a non-zero power of two, so terrains union into a
/// [`TerrainMask`](crate::terrain_mask::TerrainMask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerrainId(u32);

impl TerrainId {
    /// Creates a `TerrainId` from a raw bit value.
    ///
    /// Panics if `value` is zero or not a power of two.
    pub const fn new(value: u32) -> Self {
        assert!(
            value != 0 && value.is_power_of_two(),
            "TerrainId must be a non-zero power of two"
        );
        Self(value)
    }
}

impl Display for TerrainId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for TerrainId {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
