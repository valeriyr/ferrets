use bevy::prelude::*;
use ferrets_simulation::{
    components::{dying::DyingComponent, order_queue::OrderQueueComponent},
    entity_index::EntityIndex,
    game_loop,
};

/// Advances every alive entity's order queue by one tick: first prepares each
/// front order, then lets suspended watchers interrupt their sub-orders, then
/// processes the front.
pub fn tick_orders(world: &mut World) {
    prepare(world);
    watch(world);
    process(world);
}

/// Flush cancelled entries and prepare the front order for every alive entity.
fn prepare(world: &mut World) {
    for entity in alive_entities(world) {
        if world.entity(entity).contains::<DyingComponent>() {
            continue;
        }
        let Some(mut queue) = world.entity_mut(entity).take::<OrderQueueComponent>() else {
            continue;
        };
        game_loop::orders::prepare_tick(entity, &mut queue, world);
        world.entity_mut(entity).insert(queue);
    }
}

/// Give every alive entity's suspended watcher (if any) a chance to interrupt
/// the running sub-order.
fn watch(world: &mut World) {
    for entity in alive_entities(world) {
        if world.entity(entity).contains::<DyingComponent>() {
            continue;
        }
        let Some(mut queue) = world.entity_mut(entity).take::<OrderQueueComponent>() else {
            continue;
        };
        game_loop::orders::watch_tick(entity, &mut queue, world);
        world.entity_mut(entity).insert(queue);
    }
}

/// Advance the front `InProcessing` order by one tick for every alive entity.
fn process(world: &mut World) {
    for entity in alive_entities(world) {
        if world.entity(entity).contains::<DyingComponent>() {
            continue;
        }
        let Some(mut queue) = world.entity_mut(entity).take::<OrderQueueComponent>() else {
            continue;
        };
        game_loop::orders::process_tick(entity, &mut queue, world);
        world.entity_mut(entity).insert(queue);
    }
}

/// Collects alive entities in deterministic simulation-id order.
///
/// Snapshot taken before processing — entities spawned during this tick are not
/// included, and entities destroyed during this tick are skipped via the
/// `DyingComponent` check in the loops above.
fn alive_entities(world: &World) -> Vec<Entity> {
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect()
}
