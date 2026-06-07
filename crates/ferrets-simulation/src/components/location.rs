//! World position, facing, and content-defined location properties for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_size::NavSize};

/// Content-defined location properties for an entity type.
///
/// `occupation` — the set of [`NavGrid`](ferrets_pathfinder::nav_grid::NavGrid) layers
/// this entity occupies. Most entities occupy a single layer; use a combined
/// [`LayerMask`] for entities that block multiple layers simultaneously.
///
/// `size` — footprint in whole grid cells.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationStaticData {
    occupation: LayerMask,
    size: NavSize,
}

/// World position and facing of an entity in fixed-point grid units.
///
/// One unit = one grid cell. Sub-unit precision supports smooth movement between
/// cells; integer values land on cell origin corners.
///
/// `facing` is the last look direction.
/// The renderer normalizes it to the nearest sprite direction (8-way, 16-way, etc.).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationComponent {
    pub position: FixedUVec2,
    pub facing: FixedVec2,
}

impl LocationStaticData {
    /// Creates a new `LocationStaticData` with the given data.
    #[inline]
    pub fn new(occupation: impl Into<LayerMask>, size: NavSize) -> Self {
        Self {
            occupation: occupation.into(),
            size,
        }
    }

    /// Returns the set of NavGrid layers this entity occupies.
    #[inline]
    pub fn occupation(&self) -> LayerMask {
        self.occupation
    }

    /// Returns the entity's footprint size in grid cells.
    #[inline]
    pub fn size(&self) -> NavSize {
        self.size
    }
}

impl LocationComponent {
    /// Creates a new `LocationComponent` with the given data.
    #[inline]
    pub fn new(position: FixedUVec2, facing: FixedVec2) -> Self {
        Self { position, facing }
    }
}
