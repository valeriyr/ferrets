//! In-flight guard state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Per-entity in-flight guard state.
#[derive(Component, Debug, Default)]
pub struct GuardComponent {
    /// `(own position, guarded position)` when the last catch-up move started.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
