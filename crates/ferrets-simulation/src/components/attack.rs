//! In-flight attack state for the weapon a body points itself.

use crate::components::chase::ChaseState;
use bevy_ecs::prelude::*;

/// Per-entity state of a fight the order lifecycle is running.
///
/// Present exactly while an Attack order is being worked, put on when it starts
/// and taken off when it ends. The turrets a body carries keep their own state,
/// which outlives any one order (see
/// [`TurretsComponent`](crate::components::turret::TurretsComponent)).
#[derive(Component, Debug, Default)]
pub struct AttackComponent {
    /// Current position inside the attack cycle. Counts up each tick: the hit lands
    /// when it reaches `damage_point`, and the cycle restarts at `attack_period`.
    pub phase: u32,
    /// The last chase round toward the target; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}
