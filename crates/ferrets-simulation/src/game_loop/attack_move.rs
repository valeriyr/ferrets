//! Attack-move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_size::CellSize;

use super::{
    acquire,
    chase::{self, Destination},
    orders::Processing,
};
use crate::{
    components::{
        attack_move::AttackMoveComponent,
        entity_stats::StatsComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    map::Map,
    order::{AttackTarget, Leash, Order},
    session::GameSession,
    simulation_id::SimulationId,
};
use ferrets_content::entity_stats::EntityStatId;

/// Called once when an AttackMove order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot move.
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if !entity_def::of(world, entity).can_move() {
        return OrderState::Finished;
    }
    world
        .entity_mut(entity)
        .insert(AttackMoveComponent::default());
    OrderState::InProcessing
}

/// Called when an AttackMove order resumes from `Suspended`. The walk marker
/// resets so the next leg re-paths from wherever the fight ended.
pub fn prepare_suspended(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if let Some(mut driver) = world.entity_mut(entity).get_mut::<AttackMoveComponent>() {
        driver.last_chase = None;
    }
    OrderState::InProcessing
}

/// Called for every AttackMove entry that has a cancel policy.
///
/// An attack-move stops immediately under both policies.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<AttackMoveComponent>();
    OrderState::Finished
}

/// Advance an AttackMove order by one tick.
///
/// Scans for a hostile first — process runs exactly when the order is between
/// legs (freshly started or just resumed after a fight or walk), which is when
/// immediate re-acquisition matters. On a hit the order suspends into a leashed
/// attack; otherwise it advances toward the destination like a move, arriving
/// or giving up through the shared chase logic.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target = order
        .attack_move_target()
        .expect("AttackMove order must have a target");

    let Some(mut driver) = world.entity_mut(entity).take::<AttackMoveComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    if let Some(attack) = engagement(world, entity) {
        world.entity_mut(entity).insert(driver);
        return Processing::suspend(attack);
    }

    let (position, size) = entity_def::footprint(world, entity);
    let projection = world.resource::<Map>().projection();
    match chase::advance(
        &mut driver.last_chase,
        projection,
        position,
        size,
        target,
        CellSize::ONE,
        0,
    ) {
        Destination::Arrived | Destination::OutOfReach => Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            world.entity_mut(entity).insert(driver);
            Processing::suspend(move_order)
        }
    }
}

/// Called each tick while this order's `Move` sub-order runs (see
/// [`super::orders::watch_tick`]): the throttled en-route scan. On a hit the
/// walk is interrupted and replaced by a leashed attack.
pub fn watch(entity: Entity, _order: &Order, front: &Order, world: &mut World) -> Option<Order> {
    if !matches!(front, Order::Move { .. }) {
        return None;
    }
    let id = entity_def::simulation_id(world, entity);
    let tick = world.resource::<GameSession>().tick();
    if !acquire::due(id, tick) {
        return None;
    }
    engagement(world, entity)
}

/// A leashed attack on the best target in acquisition range, if the entity is
/// armed and one exists. The leash anchors where the entity stands now.
pub(super) fn engagement(world: &World, entity: Entity) -> Option<Order> {
    let acquire_range = world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective_as_u32(EntityStatId::ACQUIRE_RANGE))?;
    let target = acquire::find_target(world, entity, entity, acquire_range)?;
    Some(leashed_attack(world, entity, target, acquire_range))
}

/// Like [`engagement`], but on a specific candidate — `None` when the
/// candidate does not qualify for the entity's acquisition scan.
pub(super) fn engagement_on(world: &World, entity: Entity, target: SimulationId) -> Option<Order> {
    let acquire_range = world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective_as_u32(EntityStatId::ACQUIRE_RANGE))?;
    if !acquire::qualifies(world, entity, entity, target, acquire_range) {
        return None;
    }
    Some(leashed_attack(world, entity, target, acquire_range))
}

fn leashed_attack(world: &World, entity: Entity, target: SimulationId, radius: u32) -> Order {
    let anchor = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    Order::Attack {
        target: AttackTarget::Entity(target),
        leash: Some(Leash { anchor, radius }),
    }
}
