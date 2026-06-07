use bevy::{ecs::system::SystemState, prelude::*};
use ferrets_simulation::{
    components::{
        dying::DyingComponent, entity_info::EntityInfoComponent, order_queue::OrderQueueComponent,
    },
    game_loop,
};

pub fn tick_orders(world: &mut World) {
    prepare(world);
    process(world);
}

/// Flush cancelled entries and prepare the front `New` order for every alive entity.
fn prepare(world: &mut World) {
    for entity in alive_ordered_entities(world) {
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

/// Advance the front `InProcessing` order by one tick for every alive entity.
fn process(world: &mut World) {
    for entity in alive_ordered_entities(world) {
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

/// Collects alive entities with order queues, sorted by simulation ID for determinism.
///
/// Snapshot taken before processing — entities spawned during this tick are not included.
fn alive_ordered_entities(world: &mut World) -> Vec<Entity> {
    let mut state = SystemState::<
        Query<(Entity, &EntityInfoComponent), (With<OrderQueueComponent>, Without<DyingComponent>)>,
    >::new(world);

    let query = state.get(&*world);
    let mut items: Vec<_> = query.iter().collect();
    items.sort_by_key(|(_, info)| info.id());
    items.into_iter().map(|(e, _)| e).collect()
}
