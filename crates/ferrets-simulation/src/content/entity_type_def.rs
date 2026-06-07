//! Definition of a single entity type — the content-level blueprint for spawning.

use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, nav_size::NavSize};

use crate::components::{location::LocationStaticData, movement::MoveStaticData};

/// Content-level blueprint for an entity type (unit, building, resource, …).
///
/// Holds the static data components that are identical for every instance of this type.
pub struct EntityTypeDef {
    /// Unique type name used to look up this definition in [`ContentRegistry`].
    pub name: String,
    /// Navigation and footprint properties shared by all instances of this type.
    pub location: LocationStaticData,
    /// Movement properties. `None` means the entity cannot move.
    pub movement: Option<MoveStaticData>,
}

impl EntityTypeDef {
    /// Creates a new definition with the given name, nav-layer occupation, and footprint size.
    pub fn new(name: impl Into<String>, occupation: impl Into<LayerMask>, size: NavSize) -> Self {
        Self {
            name: name.into(),
            location: LocationStaticData::new(occupation, size),
            movement: None,
        }
    }

    /// Enables movement for this entity type at the given speed (grid units per tick).
    pub fn with_movement(mut self, speed: FixedU64) -> Self {
        self.movement = Some(MoveStaticData::new(speed));
        self
    }
}
