//! In-flight follow state for simulation entities.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity in-flight follow state.
#[derive(Component, Debug, Default)]
pub struct FollowComponent {
    /// `(own position, target position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: ChaseState,
}
