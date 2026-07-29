//! Attack order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;
use ferrets_pathfinder::{astar, nav_pos::NavPos};

use super::chase::{self, Destination};
use super::orders::Processing;
use crate::{
    components::{
        attack::AttackComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        stats::{StatId, StatsComponent},
        tags::TagsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    session::GameSession,
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

    if !entity_def::of(world, entity).can_attack() {
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
    let stats = world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("attackers have a stat store");
    let range = stats.effective_as_u32(StatId::ATTACK_RANGE).unwrap();
    let damage = stats.effective(StatId::DAMAGE).unwrap();
    let attack_period = stats.effective_as_u32(StatId::ATTACK_PERIOD).unwrap();
    // Registration keeps the authored damage point inside the authored cycle, but
    // the two stats take modifiers independently, so a shortened cycle can leave
    // the hit beyond its end — where the phase counter would never reach it.
    let damage_point = stats
        .effective_as_u32(StatId::DAMAGE_POINT)
        .unwrap()
        .min(attack_period);

    if let Some(leash) = order.attack_leash() {
        let target_size = entity_def::of(world, target).location.unwrap().size();
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

    match chase::advance_to_entity(
        &mut attack_component.last_chase,
        world,
        position,
        target,
        range,
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

    if attack_component.phase == damage_point {
        let attacker_id = world
            .entity(entity)
            .get::<EntityInfoComponent>()
            .unwrap()
            .id();
        let tick = world.resource::<GameSession>().tick();
        let final_damage = modify_damage(world, entity, target, damage);
        let mut target_died = false;
        if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
            health.apply_damage(final_damage);
            health.record_hit(attacker_id, tick);
            target_died = health.is_dead();
        }
        if target_died {
            spawn::destroy_entity(world, target);
        }
    }

    if attack_component.phase == attack_period {
        attack_component.phase = 0;
    }

    world.entity_mut(entity).insert(attack_component);
    Processing::state(OrderState::InProcessing)
}

/// The damage one hit deals to `target`: the attacker's effective damage plus any
/// bonus-vs-tag/type, less the target's flat armor, floored at `1` so no unit is
/// invulnerable. This is the armor & damage-class calculation.
fn modify_damage(world: &World, attacker: Entity, target: Entity, base: FixedU64) -> FixedU64 {
    let target_ref = world.entity(target);
    let target_type = target_ref
        .get::<EntityInfoComponent>()
        .map_or("", |info| info.type_name());
    let bonus = entity_def::of(world, attacker)
        .bonus_against(target_type, target_ref.get::<TagsComponent>());
    let armor = target_ref
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(StatId::ARMOR))
        .unwrap_or(FixedU64::ZERO);
    let dealt = base + FixedU64::from_num(bonus);
    dealt.saturating_sub(armor).max(FixedU64::ONE)
}
