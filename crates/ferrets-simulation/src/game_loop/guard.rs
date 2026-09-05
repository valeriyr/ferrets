//! Guard order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    acquire, attack_move,
    chase::{self, Destination},
    orders::{self, Processing, Refusal},
};
use crate::{
    components::owner,
    components::{
        guard::GuardComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    session::GameSession,
};

/// How close a guard stays to the entity it guards, in grid cells.
const GUARD_DISTANCE: u32 = 2;

/// Whether `entity` may start a Guard: its type moves and it operates, and the
/// ward is there and not hostile.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let ward = order
        .guard_target()
        .expect("Guard order must have a target");
    if !entity_def::of(world, entity).can_move() {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    let Some(ward) = world.resource::<EntityIndex>().interactable(world, ward) else {
        return Err(Refusal::TargetGone);
    };
    if owner::are_hostile(
        world.resource::<GameSession>(),
        entity_def::owner(world, entity),
        entity_def::owner(world, ward),
    ) {
        return Err(Refusal::TargetUnfit);
    }
    Ok(())
}

/// Called once when a Guard order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
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

    let id = entity_def::simulation_id(world, entity);
    let tick = world.resource::<GameSession>().tick();
    if acquire::due(id, tick)
        && let Some(attack) = engagement(world, entity, ward)
    {
        world.entity_mut(entity).insert(driver);
        return Processing::suspend(attack);
    }

    match chase::advance_to_entity(&mut driver.last_chase, world, entity, ward, GUARD_DISTANCE) {
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
    let id = entity_def::simulation_id(world, entity);
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
    // A guard walks on its ward's behalf, so it stops for anything it carries a
    // weapon for, as far off as any of it notices.
    let reach = entity_def::weapon_targets(world, entity);
    let notice = entity_def::notice_range(world, entity);
    if let Some(attacker) = acquire::fresh_attacker(world, ward)
        && let Some(attack) = attack_move::engagement_on(world, entity, reach, notice, attacker)
    {
        return Some(attack);
    }
    attack_move::engagement(world, entity, reach, notice)
}
