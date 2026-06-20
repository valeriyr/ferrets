//! World position, facing, and content-defined location properties for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_size::NavSize};

/// Whether an entity's footprint blocks the cells it stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solidity {
    /// The footprint claims the entity's layers; others collide with it.
    Solid,
    /// The entity is placed and collides on its layers, but never claims them;
    /// others pass through it freely, and passable entities can share cells.
    Passable,
}

/// Content-defined location properties for an entity type.
///
/// `occupation` — the set of [`NavGrid`](ferrets_pathfinder::nav_grid::NavGrid) layers
/// this entity lives on: cells must be free on these layers for it to stand or
/// path there. Most entities use a single layer; use a combined [`LayerMask`]
/// for entities that interact with multiple layers simultaneously.
///
/// `solidity` — whether the footprint also claims those layers.
///
/// `size` — footprint in whole grid cells.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationStaticData {
    occupation: LayerMask,
    solidity: Solidity,
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
    ///
    /// Panics if `occupation` is empty or `size` has a zero dimension.
    #[inline]
    pub fn new(occupation: impl Into<LayerMask>, size: NavSize, solidity: Solidity) -> Self {
        let occupation = occupation.into();

        assert!(
            occupation != LayerMask::EMPTY,
            "occupation must not be empty"
        );
        assert!(
            size.width > 0 && size.height > 0,
            "size dimensions must be greater than 0"
        );

        Self {
            occupation,
            solidity,
            size,
        }
    }

    /// Returns the set of NavGrid layers this entity lives on.
    #[inline]
    pub fn occupation(&self) -> LayerMask {
        self.occupation
    }

    /// Returns whether the footprint claims the entity's layers.
    #[inline]
    pub fn solidity(&self) -> Solidity {
        self.solidity
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
