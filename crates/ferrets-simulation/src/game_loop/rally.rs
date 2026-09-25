//! Dispatching a unit per its holder's rally point, once the tick's orders
//! have run. The owing side lives in [`crate::rally`].

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_size::CellSize;

use super::executor;
use crate::{
    components::{
        order_queue::OrderQueueComponent,
        rally::{RallyDueComponent, RallyPointComponent, RallyTarget},
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    rally, visibility,
};

/// Drops the entity rally target of every holder whose owner can no longer
/// make it out, and gives every unit owed a rally dispatch its order, in
/// ascending simulation-id order.
///
/// A rally set on an entity lapses with the sight of it, as an order aimed at
/// it would: the holder rallies nowhere until it is set again. A position
/// rallies as a plain move; an entity resolves like a send-to-entity intent
/// from the unit's own perspective (e.g. a worker harvests a source, a
/// soldier attacks a hostile), and one the unit's owner cannot make out
/// issues nothing, as a target gone by now issues nothing.
pub fn dispatch(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if lapses(world, entity) {
            world
                .entity_mut(entity)
                .get_mut::<RallyPointComponent>()
                .expect("a holder with a rally target carries the rally point")
                .0 = None;
        }
        let Some(due) = world.entity_mut(entity).take::<RallyDueComponent>() else {
            continue;
        };
        if let Some(order) = resolve(world, entity, due.target) {
            world
                .entity_mut(entity)
                .get_mut::<OrderQueueComponent>()
                .expect("every spawned entity carries an order queue")
                .push(order, None);
        }
    }
}

/// Whether `holder`'s rally point names an entity its owner can no longer
/// make out — gone, hidden away, in fog or concealed beyond what the owner's
/// seat detects. A holder rallying to a position, or to nothing, and one
/// nobody owns never lapse: an ownerless holder has no side to see with.
fn lapses(world: &World, holder: Entity) -> bool {
    let Some(RallyTarget::Entity(id)) = rally::target_of(world, holder) else {
        return false;
    };
    let Some(owner) = entity_def::owner(world, holder) else {
        return false;
    };
    visibility::interactable_to(world, owner, id).is_none()
}

/// The order a rally `target` gives a freshly released `unit`, if any.
fn resolve(world: &mut World, unit: Entity, target: RallyTarget) -> Option<Order> {
    match target {
        RallyTarget::Position(position) => Some(Order::Move {
            target: position,
            size: CellSize::ONE,
            range: 0,
        }),
        RallyTarget::Entity(id) => {
            // Judged as the send-to-entity command is: a target the unit's
            // owner cannot make out is not one it may be sent after.
            if let Some(owner) = entity_def::owner(world, unit)
                && visibility::interactable_to(world, owner, id).is_none()
            {
                return None;
            }
            executor::resolve_send_to_entity(world, unit, id)
        }
    }
}
