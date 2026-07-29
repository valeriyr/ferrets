//! Follow order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::chase::{self, Destination};
use super::orders::Processing;
use crate::{
    components::{
        follow::FollowComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
};

/// How close the entity stays to its follow target, in grid cells.
const FOLLOW_DISTANCE: u32 = 1;

/// Called once when a Follow order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot move or the target is gone.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if !entity_def::of(world, entity).can_move() {
        return OrderState::Finished;
    }
    let target = order
        .follow_target()
        .expect("Follow order must have a target");
    if world
        .resource::<EntityIndex>()
        .interactable(world, target)
        .is_none()
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(FollowComponent::default());
    OrderState::InProcessing
}

/// Called when a Follow order resumes from `Suspended` (its chase move just
/// finished). The driver component survives suspension; validation happens in
/// [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Follow entry that has a cancel policy.
///
/// A follow stops immediately under both policies.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<FollowComponent>();
    OrderState::Finished
}

/// Advance a Follow order by one tick.
///
/// Each tick:
/// 1. If the target is gone, the order finishes.
/// 2. If the target is out of follow distance, a chase move toward its current
///    position is requested as a sub-order (the entry suspends). The order
///    finishes instead when the chase can make no progress — the target is
///    unreachable.
/// 3. Otherwise the entity idles in place; the order never finishes on its own
///    while the target is alive.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let Some(mut follow_component) = world.entity_mut(entity).take::<FollowComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let target = order
        .follow_target()
        .expect("Follow order must have a target");
    let Some(target) = world.resource::<EntityIndex>().interactable(world, target) else {
        return Processing::state(OrderState::Finished);
    };

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;

    match chase::advance_to_entity(
        &mut follow_component.last_chase,
        world,
        position,
        target,
        FOLLOW_DISTANCE,
    ) {
        Destination::OutOfReach => Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            world.entity_mut(entity).insert(follow_component);
            Processing::suspend(move_order)
        }
        Destination::Arrived => {
            chase::face_entity(world, entity, target);
            world.entity_mut(entity).insert(follow_component);
            Processing::state(OrderState::InProcessing)
        }
    }
}
