//! In-flight attack-move state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Per-entity in-flight attack-move state.
#[derive(Component, Debug, Default)]
pub struct AttackMoveComponent {
    /// `(own position, destination)` when the last walk leg started.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
