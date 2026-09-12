//! Dispatching a unit per its holder's rally point, once the tick's orders
//! have run. The owing side lives in [`crate::rally`].

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_size::CellSize;

use super::executor;
use crate::{
    components::{
        order_queue::OrderQueueComponent,
        rally::{RallyDueComponent, RallyTarget},
    },
    entity_index::EntityIndex,
    order::Order,
};

/// Gives every unit owed a rally dispatch its order, in ascending
/// simulation-id order.
///
/// A position rallies as a plain move; an entity resolves like a send-to-entity
/// intent from the unit's own perspective (e.g. a worker harvests a source, a
/// soldier attacks a hostile). A rally target gone by now issues nothing.
pub fn dispatch(world: &mut World) {
    for (_, unit) in world.resource::<EntityIndex>().alive_entries() {
        let Some(due) = world.entity_mut(unit).take::<RallyDueComponent>() else {
            continue;
        };
        if let Some(order) = resolve(world, unit, due.target) {
            world
                .entity_mut(unit)
                .get_mut::<OrderQueueComponent>()
                .expect("every spawned entity carries an order queue")
                .push(order, None);
        }
    }
}

/// The order a rally `target` gives a freshly released `unit`, if any.
fn resolve(world: &mut World, unit: Entity, target: RallyTarget) -> Option<Order> {
    match target {
        RallyTarget::Position(position) => Some(Order::Move {
            target: position,
            size: CellSize::ONE,
            range: 0,
        }),
        RallyTarget::Entity(id) => executor::resolve_send_to_entity(world, unit, id),
    }
}
