//! In-flight construction state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;
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
    /// The last chase round toward the site; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}
