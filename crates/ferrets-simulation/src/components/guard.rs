//! In-flight guard state for simulation entities.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity in-flight guard state.
#[derive(Component, Debug, Default)]
pub struct GuardComponent {
    /// The last catch-up move's chase round toward the guarded entity;
    /// identical rounds accumulate until the chase gives up (see
    /// [`ChaseState`]).
    pub last_chase: ChaseState,
}
