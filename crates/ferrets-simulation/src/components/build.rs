//! In-flight construction state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// Marks a building whose construction is still in progress.
///
/// The progress lives on the site rather than on any one builder, so several
/// builders advance the same work and what they have raised so far outlives the
/// one that started it.
#[derive(Component, Debug, Default)]
pub struct UnderConstructionComponent {
    /// Ticks of work put into the site.
    pub progress: u32,
    /// The builders working the site right now. Empty for a site nobody has taken
    /// up: the marker says the building is unfinished, not that anyone is on it.
    pub builders: BTreeSet<SimulationId>,
}

/// Per-entity in-flight construction state.
#[derive(Component, Debug, Default)]
pub struct BuildComponent {
    /// The building being constructed, once it has been placed on the map or an
    /// already-started site has been taken up.
    pub building: Option<SimulationId>,
    /// `(own position, site position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
