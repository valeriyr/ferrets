//! A terrain mask — a set of terrains.

use std::{
    fmt::Display,
    ops::{BitAnd, BitOr, BitOrAssign, Deref},
};

use crate::terrain_id::TerrainId;

/// A set of terrains, one bit per [`TerrainId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerrainMask(u32);

impl TerrainMask {
    /// No terrain.
    pub const EMPTY: Self = Self(0);

    /// Whether the two sets share a terrain.
    pub fn meets(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl From<u32> for TerrainMask {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<TerrainId> for TerrainMask {
    fn from(id: TerrainId) -> Self {
        Self(*id)
    }
}

impl Display for TerrainMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#b}", self.0)
    }
}

impl Deref for TerrainMask {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl BitAnd for TerrainMask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitOr for TerrainMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<TerrainId> for TerrainMask {
    type Output = Self;

    fn bitor(self, rhs: TerrainId) -> Self {
        Self(self.0 | *rhs)
    }
}

impl BitOrAssign<TerrainId> for TerrainMask {
    fn bitor_assign(&mut self, rhs: TerrainId) {
        self.0 |= *rhs;
    }
}
