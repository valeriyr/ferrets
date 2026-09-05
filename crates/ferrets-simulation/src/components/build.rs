//! In-flight construction state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;
use crate::simulation_id::SimulationId;

/// How a construction site's progress is advanced: by the build orders of the
/// builders on it, or by the site itself once a builder that left it
/// unattended placed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteWork {
    /// Builders are on the site, hidden inside it or beside it, and their build
    /// orders advance the progress, one tick per tick each. The site never
    /// advances on its own.
    Crew {
        /// The builders whose build orders are on the site right now.
        builders: BTreeSet<SimulationId>,
    },
    /// No build order is on the site; it advances itself one tick per tick,
    /// and takes no crew.
    Unattended {
        /// The builder that placed the site.
        founder: SimulationId,
    },
}

/// Marks a building whose construction is still in progress.
///
/// The progress lives on the site rather than on any one builder, so several
/// builders advance the same work and what they have raised so far outlives the
/// one that started it.
#[derive(Component, Debug)]
pub struct UnderConstructionComponent {
    /// Ticks of work put into the site.
    pub progress: u32,
    /// How the progress is advanced, and by whom the site was placed.
    pub work: SiteWork,
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
