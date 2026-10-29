//! In-flight construction state for builders, and what a building raised
//! over a resource source puts back.

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;

use crate::{
    components::{chase::ChaseState, patience::Patience},
    simulation_id::SimulationId,
};
use ferrets_content::entity_type_def::EntityTypeId;

/// How far a Build order has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStage {
    /// No site yet: on its way to the ground, or waiting on bodies making way
    /// off it.
    Founding(Patience),
    /// On its way to a site under way, not taken up yet.
    Joining,
    /// The site it raised, or an already-started one it took up.
    Raising(SimulationId),
}

/// A Build order's construction state on this entity.
#[derive(Component, Debug)]
pub struct BuildComponent {
    /// The last chase round toward the site; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
    /// How far the order has come.
    pub stage: BuildStage,
}

impl BuildComponent {
    /// A Build order on its way to join a site, not chasing yet.
    pub fn joining() -> Self {
        Self {
            last_chase: None,
            stage: BuildStage::Joining,
        }
    }

    /// A Build order on its way to found a site, not chasing yet and not
    /// waited yet.
    pub fn founding() -> Self {
        Self {
            last_chase: None,
            stage: BuildStage::Founding(Patience::starting()),
        }
    }
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
