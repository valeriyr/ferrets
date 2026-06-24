//! Attack order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::chase::{self, Destination};
use super::orders::Processing;
use crate::{
    components::{
        attack::{AttackComponent, AttackStaticData},
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_index::EntityIndex,
    order::Order,
    spawn,
};

/// Called once when an Attack order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot attack or the target is no longer alive.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target = order
        .attack_target()
        .expect("Attack order must have a target");

    if !world.entity(entity).contains::<AttackStaticData>() {
        return OrderState::Finished;
    }
    if world
        .resource::<EntityIndex>()
        .interactable(world, target)
        .is_none()
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(AttackComponent::default());
    OrderState::InProcessing
}

/// Called when an Attack order resumes from `Suspended` (its chase move just
/// finished). The driver component survives suspension; validation happens in
/// [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Attack entry that has a cancel policy.
///
/// An attack stops immediately under both policies: the driver component is
/// removed and the entry finishes. A swing in progress is simply abandoned.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<AttackComponent>();
    OrderState::Finished
}

/// Advance an Attack order by one tick.
///
/// Each tick:
/// 1. If the target is gone or dying, the order finishes.
/// 2. If the target is out of range, the swing resets and a chase move toward the
///    target's current position is requested as a sub-order (the entry suspends).
///    The order finishes instead when the previous chase ended without the entity
///    getting any closer — the target is unreachable.
/// 3. Otherwise the entity faces the target and the swing advances: the hit lands
///    when the phase reaches `aiming`, and the cycle restarts after `reloading`
///    more ticks.
///
/// A target killed by the landed hit starts dying immediately; the order itself
/// finishes on the next tick when the target is no longer alive.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target_id = order
        .attack_target()
        .expect("Attack order must have a target");

    let Some(mut attack_component) = world.entity_mut(entity).take::<AttackComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return Processing::state(OrderState::Finished);
    };

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let target_position = world
        .entity(target)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let attack_data = *world.entity(entity).get::<AttackStaticData>().unwrap();

    match chase::advance_to_entity(
        &mut attack_component.last_chase,
        world,
        position,
        target,
        attack_data.range(),
    ) {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            // The swing resets while out of range.
            attack_component.phase = 0;
            world.entity_mut(entity).insert(attack_component);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }

    chase::face(world, entity, target_position);

    attack_component.phase += 1;

    if attack_component.phase == attack_data.aiming() {
        let mut target_died = false;
        if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
            health.apply_damage(attack_data.damage());
            target_died = health.is_dead();
        }
        if target_died {
            spawn::destroy_entity(world, target);
        }
    }

    if attack_component.phase == attack_data.aiming() + attack_data.reloading() {
        attack_component.phase = 0;
    }

    world.entity_mut(entity).insert(attack_component);
    Processing::state(OrderState::InProcessing)
}
