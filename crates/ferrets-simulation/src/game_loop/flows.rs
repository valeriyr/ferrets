//! Pools flowing each tick: up by their regeneration, down by their drain.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_stats::EntityStatId,
    pool_def::{PoolDef, PoolId},
    registry::ContentRegistry,
};
use ferrets_math::FixedU64;

use crate::{
    components::{entity_stats::StatsComponent, pools},
    entity_def,
    entity_index::EntityIndex,
    events::DeathCause,
    spawn,
};

/// Moves each energy pool by one tick by `energy_regen` net of
/// `energy_drain`, held between empty and `max_energy`.
///
/// Runs over the alive index, so the dying are already excluded. A pool also
/// settles back under a ceiling a debuff has lowered.
pub fn process_energy_flow(world: &mut World) {
    let energy_pool = *world.resource::<ContentRegistry>().pool_def(PoolId::ENERGY);
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !entity_def::has_pool(world, entity, PoolId::ENERGY) {
            continue;
        }
        flow(world, entity, PoolId::ENERGY, energy_pool);
    }
}

/// Moves each health pool by one tick by `health_regen` net of
/// `health_drain`, held under `max_health`.
///
/// Runs over the alive index, so the dying are already excluded; a site under
/// construction flows like anything else. A pool also settles back under a
/// ceiling a debuff has lowered, and one the drain empties dies of it.
pub fn process_health_flow(world: &mut World) {
    let health_pool = *world.resource::<ContentRegistry>().pool_def(PoolId::HEALTH);
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        // Nothing brings an entity back, whatever its regeneration says.
        match entity_def::pool_value(world, entity, PoolId::HEALTH) {
            Some(health) if health > FixedU64::ZERO => {}
            Some(_) | None => continue,
        }
        // A health pool this pass drains to nothing dies of it, with nobody
        // to blame: a structure withering off the field that sustains it, a
        // building burning down under its line. A pool that was already empty
        // is left where the opening rule left it.
        if flow(world, entity, PoolId::HEALTH, health_pool) == FixedU64::ZERO {
            spawn::despawn_entity(world, entity, DeathCause::Decayed);
        }
    }
}

/// Moves `entity`'s `pool` by one tick by its regeneration net of its drain,
/// held between empty and its maximum, and answers what it holds after.
fn flow(world: &mut World, entity: Entity, pool: PoolId, def: PoolDef) -> FixedU64 {
    // A pool is only ever seeded against its maximum, beside which the store
    // carries the pool's rates.
    let stats = world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("a pool implies the store it was seeded into");
    let effective = |stat: EntityStatId| {
        stats
            .effective(stat)
            .expect("a declared pool seeds all its stats")
    };
    let maximum = effective(def.maximum_stat());
    let regen = effective(def.regen_stat());
    let drain = effective(def.drain_stat());
    pools::flow(world, entity, pool, maximum, regen, drain)
}
