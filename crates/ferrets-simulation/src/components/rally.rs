//! Rally point state for entities that release units, and for movers still
//! rising, which go to theirs once risen.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::simulation_id::SimulationId;

/// Where an entity's rally point sends the units it releases, or the mover
/// itself once it has risen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RallyTarget {
    /// Walk to a world position.
    Position(FixedUVec2),
    /// Resolve the intent against an entity when the unit spawns (e.g. harvest
    /// a source, attack a hostile). A target gone by then, or one the unit's
    /// owner cannot make out, leaves the unit at its spawn cell.
    Entity(SimulationId),
}

/// The entity's rally point; `None` leaves released or risen units where they
/// stand.
#[derive(Component, Debug, Default)]
pub struct RallyPointComponent(pub Option<RallyTarget>);

/// A unit just released by a holder with a rally point set, or a mover just
/// risen with one, due the dispatch to its target.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct RallyDueComponent {
    /// Where the unit is sent.
    pub target: RallyTarget,
}
