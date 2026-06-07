use std::collections::HashMap;

use bevy::prelude::*;
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, location::LocationComponent,
        order_queue::OrderQueueComponent,
    },
    game_loop,
    input::InputFrames,
    selection::Selection,
    session::GameSession,
};

pub fn command_executor(
    mut pending: ResMut<InputFrames>,
    mut selection: ResMut<Selection>,
    session: Res<GameSession>,
    info_query: Query<(Entity, &EntityInfoComponent, &LocationComponent)>,
    mut queue_query: Query<(Entity, &mut OrderQueueComponent)>,
) {
    let mut sim_entities = HashMap::new();
    let mut positions = HashMap::new();
    for (entity, info, loc) in &info_query {
        sim_entities.insert(info.id(), entity);
        positions.insert(info.id(), loc.position);
    }

    let mut queues_vec: Vec<(Entity, Mut<OrderQueueComponent>)> = queue_query.iter_mut().collect();
    let mut queues: HashMap<Entity, &mut OrderQueueComponent> =
        queues_vec.iter_mut().map(|(e, q)| (*e, &mut **q)).collect();

    game_loop::executor::tick(
        &mut pending,
        &mut selection,
        session.tick(),
        &sim_entities,
        &positions,
        &mut queues,
    );
}
