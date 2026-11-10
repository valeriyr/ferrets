//! Attack-move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    acquire::{self, Pursuit},
    chase::{self, Destination},
    movement,
    orders::{Processing, Refusal},
};
use crate::{
    components::{
        attack_move::{AttackMoveComponent, AttackMoveStage},
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    order::{AttackTarget, Leash, Order},
    session::GameSession,
    simulation_id::SimulationId,
};
use ferrets_nav_grid::layer_mask::LayerMask;

/// Whether `entity` may start an AttackMove (see [`movement::may_walk`]).
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    movement::may_walk(world, entity)
}

/// Called once when an AttackMove order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    world
        .entity_mut(entity)
        .insert(AttackMoveComponent::deciding());
    OrderState::InProcessing
}

/// Called when an AttackMove order resumes from `Suspended`. A walk that
/// ended on its own ends the order where it left the body, as a Move would;
/// after a fight the order decides its next stage from wherever the fight
/// ended.
pub fn prepare_suspended(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    let Some(stage) = world
        .get::<AttackMoveComponent>(entity)
        .map(|driver| driver.stage)
    else {
        return OrderState::InProcessing;
    };
    match stage {
        AttackMoveStage::Walking => {
            world.entity_mut(entity).remove::<AttackMoveComponent>();
            OrderState::Finished
        }
        AttackMoveStage::Fighting => {
            world
                .get_mut::<AttackMoveComponent>(entity)
                .expect("the driver was read above")
                .stage = AttackMoveStage::Deciding;
            OrderState::InProcessing
        }
        AttackMoveStage::Deciding => {
            unreachable!("an attack-move suspends only into a walk or a fight")
        }
    }
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
) -> Processing {
    world.entity_mut(entity).remove::<AttackMoveComponent>();
    Processing::state(OrderState::Finished)
}

/// Whether an AttackMove can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance an AttackMove order by one tick.
///
/// Scans for a hostile first — process runs exactly when the order is deciding
/// (freshly started or just back from a fight). On a hit the order suspends
/// into a leashed attack; otherwise it walks toward the destination as a move
/// does, ending when it already stands there or is held and cannot walk.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target = order
        .attack_move_target()
        .expect("AttackMove order must have a target");

    let Some(mut driver) = world.entity_mut(entity).take::<AttackMoveComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    if let Some(attack) = engagement(
        world,
        entity,
        entity_def::weapon_targets(world, entity),
        entity_def::notice_range(world, entity),
    ) {
        driver.stage = AttackMoveStage::Fighting;
        world.entity_mut(entity).insert(driver);
        return Processing::suspend(attack);
    }

    match chase::advance_to_point(world, entity, target) {
        Destination::Arrived | Destination::OutOfReach => Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            driver.stage = AttackMoveStage::Walking;
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
    // Walking on to fight is the whole body's business, so it stops for anything
    // any of its weapons can answer, as far off as any of them notices.
    let attack = engagement(
        world,
        entity,
        entity_def::weapon_targets(world, entity),
        entity_def::notice_range(world, entity),
    )?;
    world
        .get_mut::<AttackMoveComponent>(entity)
        .expect("a walking attack-move carries its driver")
        .stage = AttackMoveStage::Fighting;
    Some(attack)
}

/// A leashed attack on the best target within `notice` cells that `targets`
/// reaches, if one exists. The leash anchors where the entity stands now.
///
/// What reach to look with, and how far, are the caller's: a walk that stops to
/// fight stops for anything the body carries as far as any of it notices, while a
/// stance engaging on its own initiative is giving its own weapon a fight and
/// leaves the turrets to theirs.
pub(super) fn engagement(
    world: &World,
    entity: Entity,
    targets: LayerMask,
    notice: u32,
) -> Option<Order> {
    let target = acquire::find_target(world, entity, targets, notice, Pursuit::Closes)?;
    Some(leashed_attack(world, entity, target, notice))
}

/// Like [`engagement`], but on a specific candidate — `None` when the
/// candidate does not qualify for the entity's acquisition scan.
pub(super) fn engagement_on(
    world: &World,
    entity: Entity,
    targets: LayerMask,
    notice: u32,
    target: SimulationId,
) -> Option<Order> {
    if !acquire::qualifies(world, entity, targets, target, notice, Pursuit::Closes) {
        return None;
    }
    Some(leashed_attack(world, entity, target, notice))
}

fn leashed_attack(world: &World, entity: Entity, target: SimulationId, radius: u32) -> Order {
    let position = entity_def::position(world, entity);
    Order::Attack {
        target: AttackTarget::Entity(target),
        leash: Some(Leash { position, radius }),
    }
}
