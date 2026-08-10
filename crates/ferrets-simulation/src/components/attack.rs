//! In-flight attack state for simulation entities.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity in-flight attack state.
#[derive(Component, Debug, Default)]
pub struct AttackComponent {
    /// Current position inside the attack cycle. Counts up each tick: the hit lands
    /// when it reaches `damage_point`, and the cycle restarts at `attack_period`.
    pub phase: u32,
    /// The last chase round toward the target; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}
