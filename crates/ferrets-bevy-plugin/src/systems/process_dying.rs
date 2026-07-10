use bevy::prelude::*;
use ferrets_simulation::{
    components::{dying::DiedComponent, order_queue::OrderQueueComponent},
    entity_index::EntityIndex,
    game_loop, spawn,
};

/// Advances the dying phase for every dying entity, and removes from the world the
/// ones whose dying phase has completed.
///
/// Mirrors the alive order loop: the only order a dying entity executes is its
/// `Die` order, preceded by the cancel flush that tears down the driver components
/// of whatever it was doing when it was destroyed.
pub fn process_dying(world: &mut World) {
    let dying = dying_entities(world);

    for &entity in &dying {
        let Some(mut queue) = world.entity_mut(entity).take::<OrderQueueComponent>() else {
            continue;
        };
        game_loop::orders::prepare_tick(entity, &mut queue, world);
        world.entity_mut(entity).insert(queue);
    }

    for &entity in &dying {
        let Some(mut queue) = world.entity_mut(entity).take::<OrderQueueComponent>() else {
            continue;
        };
        game_loop::orders::process_tick(entity, &mut queue, world);
        world.entity_mut(entity).insert(queue);

        if world.entity(entity).contains::<DiedComponent>() {
            spawn::remove_dead_entity(world, entity);
        }
    }
}

/// Collects dying entities in deterministic simulation-id order.
fn dying_entities(world: &World) -> Vec<Entity> {
    world
        .resource::<EntityIndex>()
        .dying_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect()
}
