//! In-flight construction state for builders, and what a building raised
//! over a resource source puts back.

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;

use crate::{components::chase::ChaseState, simulation_id::SimulationId};
use ferrets_content::entity_type_def::EntityTypeId;

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
