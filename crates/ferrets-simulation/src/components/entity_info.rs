//! Core entity identity — present on every simulation entity.

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;

/// Every entity carries this component. [`SimulationId`] is identical on all clients
/// because entities are spawned in the same deterministic order everywhere.
///
/// `type_name` is a unique content-registry key (e.g. `"footman"`) — not a display name.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct EntityInfoComponent {
    /// Unique ID for this entity.
    id: SimulationId,
    /// Unique content-registry key for this entity.
    type_name: String,
}

impl EntityInfoComponent {
    /// Creates a new `EntityInfoComponent` with the given data.
    #[inline]
    pub fn new(id: SimulationId, type_name: impl Into<String>) -> Self {
        Self {
            id,
            type_name: type_name.into(),
        }
    }

    #[inline]
    pub fn id(&self) -> SimulationId {
        self.id
    }

    #[inline]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }
}
