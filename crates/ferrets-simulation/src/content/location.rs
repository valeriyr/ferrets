//! Content-defined location property struct and its footprint solidity.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationDef {
    /// The set of [`NavGrid`](ferrets_pathfinder::nav_grid::NavGrid) layers this
    /// entity lives on: cells must be free on these layers for it to stand or
    /// path there. Most entities use a single layer; use a combined [`LayerMask`]
    /// for entities that interact with multiple layers simultaneously.
    occupation: LayerMask,
    /// Whether the footprint also claims the occupied layers.
    solidity: Solidity,
    /// Footprint in whole grid cells.
    size: NavSize,
}

impl LocationDef {
    /// Creates a new `LocationDef` with the given data.
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
