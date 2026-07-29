//! Guard order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{acquire, attack_move, chase, chase::Destination, orders::Processing};
use crate::{
    components::{
        entity_info::EntityInfoComponent,
        guard::GuardComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    session::GameSession,
};

/// How close a guard stays to the entity it guards, in grid cells.
const GUARD_DISTANCE: u32 = 2;

/// Called once when a Guard order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot move or the guarded entity is gone.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if !entity_def::of(world, entity).can_move() {
        return OrderState::Finished;
    }
    let ward = order
        .guard_target()
        .expect("Guard order must have a target");
    if world
        .resource::<EntityIndex>()
        .interactable(world, ward)
        .is_none()
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(GuardComponent::default());
    OrderState::InProcessing
}

/// Called when a Guard order resumes from `Suspended`. The catch-up marker
/// resets so the next move re-paths from wherever the fight ended.
pub fn prepare_suspended(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if let Some(mut driver) = world.entity_mut(entity).get_mut::<GuardComponent>() {
        driver.last_chase = None;
    }
    OrderState::InProcessing
}

/// Called for every Guard entry that has a cancel policy.
///
/// A guard stops immediately under both policies.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<GuardComponent>();
    OrderState::Finished
}

/// Advance a Guard order by one tick.
///
/// Each tick:
/// 1. If the guarded entity is gone, the order finishes.
/// 2. On due ticks, scan for an engagement — whoever recently hit the ward
///    first, then the guard's own surroundings — and suspend into a leashed
///    attack on a hit.
/// 3. Otherwise stay within guard distance of the ward, chasing it as it
///    moves; an unreachable ward finishes the order.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let ward_id = order
        .guard_target()
        .expect("Guard order must have a target");

    let Some(mut driver) = world.entity_mut(entity).take::<GuardComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let Some(ward) = world.resource::<EntityIndex>().interactable(world, ward_id) else {
        return Processing::state(OrderState::Finished);
    };

    let id = world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .unwrap()
        .id();
    let tick = world.resource::<GameSession>().tick();
    if acquire::due(id, tick)
        && let Some(attack) = engagement(world, entity, ward)
    {
        world.entity_mut(entity).insert(driver);
        return Processing::suspend(attack);
    }

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    match chase::advance_to_entity(
        &mut driver.last_chase,
        world,
        position,
        ward,
        GUARD_DISTANCE,
    ) {
        Destination::OutOfReach => Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            world.entity_mut(entity).insert(driver);
            Processing::suspend(move_order)
        }
        Destination::Arrived => {
            chase::face_entity(world, entity, ward);
            world.entity_mut(entity).insert(driver);
            Processing::state(OrderState::InProcessing)
        }
    }
}

/// Called each tick while this order's `Move` sub-order runs (see
/// [`super::orders::watch_tick`]): scan while catching up to the ward.
pub fn watch(entity: Entity, order: &Order, front: &Order, world: &mut World) -> Option<Order> {
    if !matches!(front, Order::Move { .. }) {
        return None;
    }
    let id = world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .unwrap()
        .id();
    let tick = world.resource::<GameSession>().tick();
    if !acquire::due(id, tick) {
        return None;
    }
    let ward = order
        .guard_target()
        .expect("Guard order must have a target");
    let ward = world.resource::<EntityIndex>().interactable(world, ward)?;
    engagement(world, entity, ward)
}

/// The guard's engagement: the ward's fresh attacker when it qualifies for
/// the guard's own acquisition scan, otherwise whatever that scan finds.
fn engagement(world: &World, entity: Entity, ward: Entity) -> Option<Order> {
    // Answer the ward's attacker first — that is what a guard exists for. The
    // hint is only a candidate, still subject to range and hostility.
    if let Some(attacker) = acquire::fresh_attacker(world, ward)
        && let Some(attack) = attack_move::engagement_on(world, entity, attacker)
    {
        return Some(attack);
    }
    attack_move::engagement(world, entity)
}
