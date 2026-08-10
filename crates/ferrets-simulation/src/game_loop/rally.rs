//! Dispatching a unit per its holder's rally point.
//!
//! Every order that releases a unit beside a rally-carrying holder — a trainer
//! finishing a unit, a transporter letting a passenger out — sends it on the
//! same way; this module keeps that dispatch in one place.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_size::CellSize;

use super::executor;
use crate::{
    components::{
        order_queue::OrderQueueComponent,
        rally::{RallyPointComponent, RallyTarget},
    },
    order::Order,
};

/// Sends a freshly released unit toward `holder`'s rally point, if one is set.
///
/// A position rallies as a plain move; an entity resolves like a send-to-entity
/// intent from the unit's own perspective (e.g. a worker harvests a source, a
/// soldier attacks a hostile). A rally target gone by release time issues
/// nothing.
pub(super) fn send(world: &mut World, holder: Entity, unit: Entity) {
    let Some(target) = world
        .entity(holder)
        .get::<RallyPointComponent>()
        .and_then(|rally| rally.0)
    else {
        return;
    };

    let order = match target {
        RallyTarget::Position(position) => Some(Order::Move {
            target: position,
            size: CellSize::ONE,
            range: 0,
        }),
        RallyTarget::Entity(id) => executor::resolve_send_to_entity(world, unit, id),
    };
    if let Some(order) = order
        && let Some(mut queue) = world.entity_mut(unit).get_mut::<OrderQueueComponent>()
    {
        queue.push(order, None);
    }
}
