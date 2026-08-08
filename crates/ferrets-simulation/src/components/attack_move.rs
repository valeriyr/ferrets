//! In-flight attack-move state for simulation entities.

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;

/// Per-entity in-flight attack-move state.
#[derive(Component, Debug, Default)]
pub struct AttackMoveComponent {
    /// `(own position, destination)` when the last walk leg started.
    pub last_chase: ChaseState,
}
