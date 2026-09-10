//! In-flight construction state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;

use crate::components::chase::ChaseState;
use crate::simulation_id::SimulationId;
use ferrets_content::entity_type_def::EntityTypeId;

/// How a construction site's progress is advanced: by the build orders of the
/// builders on it, by the site itself once a builder that left it unattended
/// placed it, or not at all while it waits for a builder to take it up again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteWork {
    /// Builders are on the site, hidden inside it, on it or beside it, and
    /// their build orders advance the progress, one tick per tick each. The
    /// site never advances on its own.
    Crew {
        /// The builders whose build orders are on the site right now.
        builders: BTreeSet<SimulationId>,
    },
    /// No build order is on the site; it advances itself one tick per tick,
    /// and takes no crew.
    Unattended {
        /// The builder the site advances on behalf of: the one that placed it,
        /// or the one that has since taken it up.
        founder: SimulationId,
    },
    /// No build order is on the site and it does not advance: its crew left
    /// it standing with the progress raised so far, and any builder that
    /// raises the type takes it up again.
    Halted,
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

impl UnderConstructionComponent {
    /// Steps the work for the `tender` offering to advance the site: a halted
    /// site is taken up as that tender's own, an unattended one whose tender
    /// has gone is halted again, and a site with a crew on it is being worked
    /// by that crew already.
    pub fn tend(&mut self, tender: Option<SimulationId>) {
        match (&self.work, tender) {
            (SiteWork::Halted, Some(founder)) => self.work = SiteWork::Unattended { founder },
            (SiteWork::Unattended { .. }, None) => self.work = SiteWork::Halted,
            (SiteWork::Crew { .. }, _)
            | (SiteWork::Halted, None)
            | (SiteWork::Unattended { .. }, Some(_)) => {}
        }
    }
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

/// Marks an entity raised over a resource source it took the remaining
/// amount of, which comes back as that source when the entity dies.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverbuiltComponent {
    /// The source type put back with what is left.
    pub uncovers: EntityTypeId,
    /// The cell the covered source's footprint was anchored at.
    pub anchor: CellPos,
}
