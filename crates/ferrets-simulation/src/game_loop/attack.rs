//! Attack order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_pathfinder::{astar, nav_pos::NavPos, nav_size::NavSize};

use super::chase::{self, Destination};
use super::impacts;
use super::orders::Processing;
use crate::{
    components::{
        attack::AttackComponent,
        entity_stats::StatsComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    content::entity_stats::EntityStatId,
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::{AttackTarget, Order},
};

/// Called once when an Attack order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot attack or the target is no longer alive.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target = order
        .attack_target()
        .expect("Attack order must have a target");

    if !entity_def::of(world, entity).can_attack() {
        return OrderState::Finished;
    }
    // A cell is always there to be shelled; only a named entity can already be gone.
    if let Some(id) = target.entity()
        && world
            .resource::<EntityIndex>()
            .interactable(world, id)
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
/// 1. If the target is gone or dying, the order finishes. A leashed attack also
///    finishes when the target has strayed beyond the leash.
/// 2. If the target is out of range, the swing resets and a chase move toward the
///    target's current position is requested as a sub-order (the entry suspends).
///    The order finishes instead when the previous chase ended without the entity
///    getting any closer — the target is unreachable.
/// 3. Otherwise the entity faces the target and the swing advances: the hit lands
///    when the phase reaches `damage_point`, and the cycle restarts at
///    `attack_period`.
///
/// A target killed by the landed hit starts dying immediately; the order itself
/// finishes on the next tick when the target is no longer alive.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target_aim = order
        .attack_target()
        .expect("Attack order must have a target");

    let Some(mut attack_component) = world.entity_mut(entity).take::<AttackComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    // A named entity must still be reachable and is chased, measured and faced by
    // its whole footprint wherever it goes; a cell is simply where it was aimed, has
    // no footprint of its own, and needs no such check.
    let (target, target_position, target_size) = match target_aim {
        AttackTarget::Entity(id) => {
            let Some(target) = world.resource::<EntityIndex>().interactable(world, id) else {
                return Processing::state(OrderState::Finished);
            };
            let (at, size) = entity_def::footprint(world, target);
            (Some(target), at, size)
        }
        AttackTarget::Position(cell) => (None, cell, NavSize::ONE),
    };

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let stats = world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("attackers have a stat store");
    let range = stats.effective_as_u32(EntityStatId::ATTACK_RANGE).unwrap();
    let damage = stats.effective(EntityStatId::DAMAGE).unwrap();
    let attack_period = stats.effective_as_u32(EntityStatId::ATTACK_PERIOD).unwrap();
    // Registration keeps the authored damage point inside the authored cycle, but
    // the two stats take modifiers independently, so a shortened cycle can leave
    // the hit beyond its end — where the phase counter would never reach it.
    let damage_point = stats
        .effective_as_u32(EntityStatId::DAMAGE_POINT)
        .unwrap()
        .min(attack_period);

    if let Some(leash) = order.attack_leash() {
        // Footprint-based like every range check, so leashes measure the same
        // distances acquisition did.
        if !astar::in_range_of_rect(
            world.resource::<Map>().projection(),
            NavPos::from(leash.anchor),
            NavPos::from(target_position),
            target_size,
            leash.radius,
        ) {
            return Processing::state(OrderState::Finished);
        }
    }

    let destination = match target {
        Some(target) => chase::advance_to_entity(
            &mut attack_component.last_chase,
            world,
            position,
            target,
            range,
        ),
        None => chase::advance(
            &mut attack_component.last_chase,
            world.resource::<Map>().projection(),
            position,
            target_position,
            target_size,
            range,
        ),
    };
    match destination {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            // The swing resets while out of range.
            attack_component.phase = 0;
            world.entity_mut(entity).insert(attack_component);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }

    chase::face(world, entity, target_position, target_size);

    attack_component.phase += 1;

    if attack_component.phase == damage_point {
        impacts::deliver(world, entity, target, target_position, damage);
    }

    if attack_component.phase == attack_period {
        attack_component.phase = 0;
    }

    world.entity_mut(entity).insert(attack_component);
    Processing::state(OrderState::InProcessing)
}
