//! In-flight attack-move state for simulation entities.

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;

/// Per-entity in-flight attack-move state.
#[derive(Component, Debug, Default)]
pub struct AttackMoveComponent {
    /// The last walk leg's chase round toward the destination; identical
    /// rounds accumulate until the walk gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}
