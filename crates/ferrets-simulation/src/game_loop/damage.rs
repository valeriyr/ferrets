//! The one place a hit turns into lost health.
//!
//! Both delivery paths — a hit that lands at the damage point and one that lands
//! from a projectile — resolve and apply damage here, so the armor and
//! damage-class rules cannot diverge between them.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    components::{entity_info::EntityInfoComponent, health::HealthComponent, tags::TagsComponent},
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent},
    session::GameSession,
    simulation_id::SimulationId,
    spawn,
};
use ferrets_content::{entity_stats::EntityStatId, entity_type_def::EntityTypeDef};

/// The damage one full-strength hit from `attacker_def` deals to `target`.
///
/// See [`resolve_scaled`] for the calculation; a direct hit is simply an unscaled
/// one.
pub fn resolve(
    world: &World,
    attacker_def: &EntityTypeDef,
    target: Entity,
    base: FixedU64,
) -> FixedU64 {
    resolve_scaled(world, attacker_def, target, base, FixedU64::ONE)
}

/// The damage one hit from `attacker_def` deals to `target` at `fraction` of full
/// strength: `base` plus any bonus against the target's tags or type, scaled by the
/// fraction, less the target's flat armor, floored at `1` so nothing is
/// invulnerable. This is the armor & damage-class calculation.
///
/// The fraction scales the bonus along with the base, because the bonus is part of
/// the hit; armor then subtracts in full, because it mitigates each hit it takes.
/// Scaling only the base would leave a blast's outer bands dealing their full bonus,
/// and subtracting armor before the fraction would make armor stronger at range.
pub fn resolve_scaled(
    world: &World,
    attacker_def: &EntityTypeDef,
    target: Entity,
    base: FixedU64,
    fraction: FixedU64,
) -> FixedU64 {
    let target_ref = world.entity(target);
    let target_type = target_ref
        .get::<EntityInfoComponent>()
        .map_or("", |info| info.type_name());
    let target_tags = target_ref.get::<TagsComponent>();
    let bonus = attacker_def.bonus_against(target_type, |tag| {
        target_tags.is_some_and(|tags| tags.contains(tag))
    });
    let armor =
        entity_def::effective_stat(world, target, EntityStatId::ARMOR).unwrap_or(FixedU64::ZERO);
    let dealt = (base + FixedU64::from_num(bonus)).saturating_mul(fraction);
    dealt.saturating_sub(armor).max(FixedU64::ONE)
}

/// Applies `amount` to `target`, recording `attacker` as the source, and starts the
/// target dying when its pool empties.
///
/// No-op for a target with no health pool.
pub fn apply(world: &mut World, attacker: SimulationId, target: Entity, amount: FixedU64) {
    let tick = world.resource::<GameSession>().tick();
    let died = {
        let mut target_mut = world.entity_mut(target);
        let Some(mut health) = target_mut.get_mut::<HealthComponent>() else {
            return;
        };
        health.drain(amount);
        health.record_hit(attacker, tick);
        health.is_dead()
    };

    // The attacker may already be gone — a shot outlives the weapon that fired
    // it — so the credit is whatever the index can still resolve, its dying
    // stage included.
    let attacker_entity = world.resource::<EntityIndex>().any(attacker);
    let attacker_owner = attacker_entity.and_then(|entity| entity_def::owner(world, entity));
    let target_owner = entity_def::owner(world, target);
    let position = entity_def::position(world, target);
    let target_id = entity_def::simulation_id(world, target);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::DamageLanded {
            target: target_id,
            target_owner,
            attacker,
            attacker_owner,
            amount,
            position,
        });

    if died {
        spawn::despawn_entity(
            world,
            target,
            DeathCause::Killed {
                by: attacker,
                by_owner: attacker_owner,
            },
        );
    }
}
