//! Core entity identity — present on every simulation entity.

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;
use ferrets_content::entity_type_def::EntityTypeId;

/// Every entity carries this component. [`SimulationId`] is identical on all clients
/// because entities are spawned in the same deterministic order everywhere.
///
/// [`EntityTypeDef`]: ferrets_content::entity_type_def::EntityTypeDef
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct EntityInfoComponent {
    /// Unique ID for this entity.
    id: SimulationId,
    /// Registry handle for this entity's type, for O(1) [`EntityTypeDef`] lookup.
    type_id: EntityTypeId,
    /// Unique content-registry key for this entity's type (e.g. `"footman"`) — not
    /// a display name — kept for debug, display, and AI views.
    type_name: String,
}

impl EntityInfoComponent {
    /// Creates a new `EntityInfoComponent` with the given data.
    #[inline]
    pub fn new(id: SimulationId, type_id: EntityTypeId, type_name: impl Into<String>) -> Self {
        Self {
            id,
            type_id,
            type_name: type_name.into(),
        }
    }

    #[inline]
    pub fn id(&self) -> SimulationId {
        self.id
    }

    /// The entity type's registry handle, for O(1) def lookup.
    #[inline]
    pub fn type_id(&self) -> EntityTypeId {
        self.type_id
    }

    #[inline]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }
}
