//! In-flight construction state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// Marks a building whose construction is still in progress.
#[derive(Component, Debug, Default)]
pub struct UnderConstructionComponent;

/// Per-entity in-flight construction state.
#[derive(Component, Debug, Default)]
pub struct BuildComponent {
    /// The building being constructed, once it has been placed on the map.
    pub building: Option<SimulationId>,
    /// Ticks spent constructing.
    pub progress: u32,
    /// `(own position, site position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
