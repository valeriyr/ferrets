//! In-flight attack state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Per-entity in-flight attack state.
#[derive(Component, Debug, Default)]
pub struct AttackComponent {
    /// Current position inside the attack cycle. Counts up each tick: the hit lands
    /// when it reaches `damage_point`, and the cycle restarts at `attack_period`.
    pub phase: u32,
    /// `(own position, target position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
