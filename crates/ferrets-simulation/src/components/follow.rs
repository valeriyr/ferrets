//! In-flight follow state for simulation entities.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity in-flight follow state.
#[derive(Component, Debug, Default)]
pub struct FollowComponent {
    /// The last chase round toward the target; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}
