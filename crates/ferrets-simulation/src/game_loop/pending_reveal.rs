//! Retry step for entities waiting on a free cell to return to the grid.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    components::{entity_info::EntityInfoComponent, pending_reveal::PendingRevealComponent},
    entity_def,
    simulation_id::SimulationId,
    spawn::{self, SetDown},
};

/// Retries the return of every entity left waiting on a free cell.
///
/// Each tick, every entity carrying a [`PendingRevealComponent`] reattempts its
/// return beside what it names; on success the entity is back on the grid,
/// and the return puts the marker away. Entities are visited in ascending [`SimulationId`]
/// order so the outcome stays deterministic across peers.
pub fn process_pending_reveals(world: &mut World) {
    let mut pending: Vec<(SimulationId, Entity, PendingRevealComponent)> = world
        .query::<(Entity, &EntityInfoComponent, &PendingRevealComponent)>()
        .iter(world)
        .map(|(entity, info, reveal)| (info.id(), entity, *reveal))
        .collect();
    pending.sort_unstable_by_key(|(id, _, _)| *id);

    for (_, entity, reveal) in pending {
        // The search beside a container that still stands is noted for when
        // it is gone.
        let last = entity_def::beside_rect(world, entity, reveal.beside);
        world
            .entity_mut(entity)
            .get_mut::<PendingRevealComponent>()
            .expect("a pending return carries its marker")
            .last = last;
        match spawn::place_back_beside(world, entity, reveal.beside, reveal.lean) {
            SetDown::Placed => {}
            // No cell yet: it waits for the next tick.
            SetDown::NoCell => {}
        }
    }
}
