//! Paying an entity cast's multi-arm cost: resources from the owner's
//! stockpile, energy and health from the entity's own pools.
//!
//! Everything priced in [`EntityCastCost`] terms charges the same way, so it
//! draws through this module: every arm is checked before any is paid, and
//! nothing ever half-charges.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    components::{energy::EnergyComponent, health::HealthComponent},
    entity_def,
    events::SpendCause,
    resources::{self, PlayerResources},
    session::player_slot::PlayerId,
};
use ferrets_content::{costs::Cost, entity_stats::EntityStatId, skills::EntityCastCost};

/// Whether every arm of the cost is payable right now. Checked in full before
/// any is paid, so a cast never half-charges. A health cost must leave the
/// entity alive: what could not be survived is refused.
pub(super) fn can_pay(
    world: &World,
    entity: Entity,
    player: PlayerId,
    costs: &[EntityCastCost],
) -> bool {
    let (resources, energy_cost, health_cost) = folded(costs);
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &resources)
    {
        return false;
    }
    let entity_ref = world.entity(entity);
    if energy_cost > FixedU64::ZERO
        && entity_ref
            .get::<EnergyComponent>()
            .is_none_or(|energy| energy.current() < energy_cost)
    {
        return false;
    }
    if health_cost > FixedU64::ZERO
        && entity_ref
            .get::<HealthComponent>()
            .is_none_or(|health| health.current() <= health_cost)
    {
        return false;
    }
    true
}

/// Draws every arm of the cost. The caller has checked [`can_pay`] in the
/// same breath, so nothing here can come up short.
pub(super) fn pay(
    world: &mut World,
    entity: Entity,
    player: PlayerId,
    costs: &[EntityCastCost],
    cause: SpendCause,
) {
    let (resources, energy_cost, health_cost) = folded(costs);
    resources::charge(world, player, resources, cause);
    let mut entity_mut = world.entity_mut(entity);
    if energy_cost > FixedU64::ZERO
        && let Some(mut energy) = entity_mut.get_mut::<EnergyComponent>()
    {
        energy.spend(energy_cost);
    }
    if health_cost > FixedU64::ZERO
        && let Some(mut health) = entity_mut.get_mut::<HealthComponent>()
    {
        health.apply_damage(health_cost);
    }
}

/// Gives back everything [`pay`] drew: resources to the owner's stockpile,
/// pool costs back into their pools, clamped at the entity's current effective
/// maxima — a pool that regenerated meanwhile keeps the overflow unspent.
pub(super) fn refund(
    world: &mut World,
    entity: Entity,
    player: PlayerId,
    costs: &[EntityCastCost],
    cause: SpendCause,
) {
    let (resources, energy_cost, health_cost) = folded(costs);
    resources::refund(world, player, resources, cause);
    let max_energy = entity_def::effective_stat(world, entity, EntityStatId::MAX_ENERGY)
        .unwrap_or(FixedU64::ZERO);
    let max_health = entity_def::effective_stat(world, entity, EntityStatId::MAX_HEALTH)
        .unwrap_or(FixedU64::ZERO);
    let mut entity_mut = world.entity_mut(entity);
    if energy_cost > FixedU64::ZERO
        && let Some(mut energy) = entity_mut.get_mut::<EnergyComponent>()
    {
        energy.regenerate(energy_cost, max_energy);
    }
    if health_cost > FixedU64::ZERO
        && let Some(mut health) = entity_mut.get_mut::<HealthComponent>()
    {
        health.heal(health_cost, max_health);
    }
}

/// The costs folded into one total per pool, so a check covers every arm that
/// draws from that pool.
fn folded(costs: &[EntityCastCost]) -> (Cost, FixedU64, FixedU64) {
    let mut resources = Cost::new();
    let mut energy = FixedU64::ZERO;
    let mut health = FixedU64::ZERO;
    for cost in costs {
        match cost {
            EntityCastCost::Resources(cost) => {
                for (kind, amount) in cost {
                    *resources.entry(kind.clone()).or_default() += amount;
                }
            }
            EntityCastCost::Energy(amount) => energy += *amount,
            EntityCastCost::Health(amount) => health += *amount,
        }
    }
    (resources, energy, health)
}
