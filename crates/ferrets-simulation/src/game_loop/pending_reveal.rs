//! Retry step for entities waiting on a free cell to reappear on the map.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    components::{entity_info::EntityInfoComponent, pending_reveal::PendingRevealComponent},
    simulation_id::SimulationId,
    spawn,
};

/// Retries the reveal of every entity left waiting on a free cell.
///
/// Each tick, every entity carrying a [`PendingRevealComponent`] reattempts its
/// reveal against its stored anchor; on success the entity is back on the map
/// and the marker is removed. Entities are visited in ascending [`SimulationId`]
/// order so the outcome stays deterministic across peers.
pub fn process_pending_reveals(world: &mut World) {
    let mut pending: Vec<(SimulationId, Entity, PendingRevealComponent)> = world
        .query::<(Entity, &EntityInfoComponent, &PendingRevealComponent)>()
        .iter(world)
        .map(|(entity, info, reveal)| (info.id(), entity, *reveal))
        .collect();
    pending.sort_unstable_by_key(|(id, _, _)| *id);

    for (_, entity, reveal) in pending {
        if spawn::reveal_entity_near(world, entity, reveal.around, reveal.around_size) {
            world.entity_mut(entity).remove::<PendingRevealComponent>();
        }
    }
}
