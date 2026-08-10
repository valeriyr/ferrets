//! Derived per-player supply accounting and the production gate over it.
//!
//! Nothing here is stored. Provided and used are summed on demand from what
//! stands on the map and what sits in training queues, so every lifecycle edge —
//! death, cancellation, a site still going up — is settled by the predicates
//! that already govern it, and effective stats are read at their current value
//! rather than cached against modifier changes.

use bevy_ecs::world::World;
use ferrets_math::FixedU64;

use crate::{
    components::{
        build::UnderConstructionComponent, entity_stats::StatsComponent, owner::OwnerComponent,
        train::TrainQueueComponent,
    },
    entity_index::EntityIndex,
    player_stats::PlayerStats,
    session::player_slot::PlayerId,
};
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, player_stats::PlayerStatId,
    registry::ContentRegistry,
};

/// The supply available to `player`: everything its standing entities provide,
/// held under the player's `max_supply` ceiling when it has one.
///
/// An entity still under construction provides nothing yet.
pub fn provided(world: &World, player: PlayerId) -> FixedU64 {
    totals(world, player).0
}

/// The supply `player` occupies: what its standing entities cost, plus what its
/// training queues have reserved.
///
/// Queued units count from the moment they are queued — reservation happens
/// where the resource cost is paid — and hand their reservation over to the
/// spawned unit, so a provider dying mid-train never strands a paid-for unit.
pub fn used(world: &World, player: PlayerId) -> FixedU64 {
    totals(world, player).1
}

/// Whether `player`'s supply admits one more instance of `def`.
///
/// A def without a supply cost is always admitted — even over a cap, which is a
/// soft state that only holds back things that would occupy more supply.
pub fn allows(world: &World, player: PlayerId, def: &EntityTypeDef) -> bool {
    let Some(cost) = def.base_stat(EntityStatId::SUPPLY_COST) else {
        return true;
    };
    let (provided, used) = totals(world, player);
    used.saturating_add(cost) <= provided
}

/// One pass over the player's entities: `(provided, used)`.
fn totals(world: &World, player: PlayerId) -> (FixedU64, FixedU64) {
    let registry = world.resource::<ContentRegistry>();
    let mut provided = FixedU64::ZERO;
    let mut used = FixedU64::ZERO;

    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_ref
            .get::<OwnerComponent>()
            .is_none_or(|owner| owner.player() != player)
        {
            continue;
        }
        // A site still going up neither feeds nor eats: it provides nothing
        // until it stands, and costs nothing until then either.
        if entity_ref.contains::<UnderConstructionComponent>() {
            continue;
        }

        if let Some(stats) = entity_ref.get::<StatsComponent>() {
            provided = provided.saturating_add(
                stats
                    .effective(EntityStatId::SUPPLY_PROVIDED)
                    .unwrap_or_default(),
            );
            used = used.saturating_add(
                stats
                    .effective(EntityStatId::SUPPLY_COST)
                    .unwrap_or_default(),
            );
        }

        // Reservations: every queued unit holds its supply already. The queue
        // stores type names, so the cost read is the def's base value — the
        // instance that will carry modifiers does not exist yet.
        if let Some(queue) = entity_ref.get::<TrainQueueComponent>() {
            for type_name in &queue.0 {
                let cost = registry
                    .entity(type_name)
                    .and_then(|def| def.base_stat(EntityStatId::SUPPLY_COST))
                    .unwrap_or_default();
                used = used.saturating_add(cost);
            }
        }
    }

    if let Some(cap) = world
        .resource::<PlayerStats>()
        .effective(player, PlayerStatId::MAX_SUPPLY)
    {
        provided = provided.min(cap);
    }

    (provided, used)
}
