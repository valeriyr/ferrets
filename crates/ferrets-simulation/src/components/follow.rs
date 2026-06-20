//! In-flight follow state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Per-entity in-flight follow state.
#[derive(Component, Debug, Default)]
pub struct FollowComponent {
    /// `(own position, target position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
