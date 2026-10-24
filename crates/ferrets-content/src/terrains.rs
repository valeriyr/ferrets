//! Which terrains a declaration names.

use ferrets_nav_grid::{terrain_id::TerrainId, terrain_mask::TerrainMask};

use crate::registry::ContentRegistry;

/// Which terrains a declaration names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrains {
    /// Every terrain.
    Any,
    /// These terrains alone.
    Only(TerrainMask),
}

impl Terrains {
    /// The mask of these terrains, any being every terrain `registry` holds.
    pub fn mask(self, registry: &ContentRegistry) -> TerrainMask {
        match self {
            Terrains::Any => registry.registered_terrains(),
            Terrains::Only(terrains) => terrains,
        }
    }

    /// Whether `terrain` is one of these.
    pub fn admits(self, terrain: TerrainId) -> bool {
        match self {
            Terrains::Any => true,
            Terrains::Only(terrains) => terrains.meets(terrain.into()),
        }
    }
}
