//! In-flight guard state for simulation entities.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity in-flight guard state.
#[derive(Component, Debug, Default)]
pub struct GuardComponent {
    /// `(own position, guarded position)` when the last catch-up move started.
    pub last_chase: ChaseState,
}
