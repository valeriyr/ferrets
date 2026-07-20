//! Rally point state for entities that emit units.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::simulation_id::SimulationId;

/// Where an entity's rally point sends freshly emitted units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RallyTarget {
    /// Walk to a world position.
    Position(FixedUVec2),
    /// Resolve the intent against an entity when the unit spawns (e.g. harvest
    /// a source, attack a hostile). A target gone by then leaves the unit at
    /// its spawn cell.
    Entity(SimulationId),
}

/// The entity's rally point; `None` leaves emitted units at their spawn cell.
#[derive(Component, Debug, Default)]
pub struct RallyPointComponent(pub Option<RallyTarget>);
