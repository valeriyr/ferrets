//! The one place a hit turns into lost health.
//!
//! Both delivery paths — a hit that lands at the damage point and one that lands
//! from a projectile — resolve and apply damage here, so the armor and
//! damage-class rules cannot diverge between them.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::components::{
    entity_info::EntityInfoComponent, health::HealthComponent, stats::StatsComponent,
    tags::TagsComponent,
};
use crate::content::{entity_type_def::EntityTypeDef, stats::StatId};
use crate::session::GameSession;
use crate::simulation_id::SimulationId;
use crate::spawn;

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
    let bonus = attacker_def.bonus_against(target_type, target_ref.get::<TagsComponent>());
    let armor = target_ref
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(StatId::ARMOR))
        .unwrap_or(FixedU64::ZERO);
    let dealt = (base + FixedU64::from_num(bonus)).saturating_mul(fraction);
    dealt.saturating_sub(armor).max(FixedU64::ONE)
}

/// Applies `amount` to `target`, recording `attacker` as the source, and starts the
/// target dying when its pool empties.
///
/// No-op for a target with no health pool.
pub fn apply(world: &mut World, attacker: SimulationId, target: Entity, amount: FixedU64) {
    let tick = world.resource::<GameSession>().tick();
    let mut died = false;
    if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
        health.apply_damage(amount);
        health.record_hit(attacker, tick);
        died = health.is_dead();
    }
    if died {
        spawn::destroy_entity(world, target);
    }
}
