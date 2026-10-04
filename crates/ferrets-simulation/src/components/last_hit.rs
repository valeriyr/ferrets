//! The most recent hit an entity took.

use bevy_ecs::{prelude::*, world::World};

use crate::simulation_id::SimulationId;

/// The most recent hit an entity took: who dealt it and when. An entity never
/// hit carries none.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LastHitComponent {
    /// The entity that dealt the damage.
    pub attacker: SimulationId,
    /// The tick the hit landed on.
    pub tick: u32,
}

/// Records that `attacker` hit `entity` on `tick`.
pub fn record(world: &mut World, entity: Entity, attacker: SimulationId, tick: u32) {
    world
        .entity_mut(entity)
        .insert(LastHitComponent { attacker, tick });
}
