//! Patrol order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::orders::Processing;
use crate::{
    components::{
        location::LocationComponent,
        movement::MoveStaticData,
        order_queue::{CancelPolicy, OrderState},
        patrol::PatrolComponent,
    },
    order::Order,
};

/// Called once when a Patrol order becomes the front `New` entry.
///
/// Records the current position as the return endpoint and returns
/// `InProcessing`, or `Finished` immediately if the entity cannot move.
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if !world.entity(entity).contains::<MoveStaticData>() {
        return OrderState::Finished;
    }
    let home = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    world.entity_mut(entity).insert(PatrolComponent {
        home,
        outbound: true,
    });
    OrderState::InProcessing
}

/// Called when a Patrol order resumes from `Suspended` (a leg just finished).
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Patrol entry that has a cancel policy.
///
/// A patrol stops immediately under both policies.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<PatrolComponent>();
    OrderState::Finished
}

/// Advance a Patrol order by one tick: suspend into the next attack-move leg,
/// alternating between the order's target and the recorded home endpoint.
/// Engaging and resuming live entirely in the leg; the patrol itself never
/// finishes on its own.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target = order
        .patrol_target()
        .expect("Patrol order must have a target");

    let mut entity_mut = world.entity_mut(entity);
    let Some(mut driver) = entity_mut.get_mut::<PatrolComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let leg_target = if driver.outbound { target } else { driver.home };
    driver.outbound = !driver.outbound;
    Processing::suspend(Order::AttackMove { target: leg_target })
}
