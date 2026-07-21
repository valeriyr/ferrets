//! In-flight patrol state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Per-entity in-flight patrol state.
#[derive(Component, Debug)]
pub struct PatrolComponent {
    /// The endpoint the patrol returns to — where the entity stood when the
    /// order started.
    pub home: FixedUVec2,
    /// `true` while the next leg heads toward the order's target, `false`
    /// while it heads back home.
    pub outbound: bool,
}
