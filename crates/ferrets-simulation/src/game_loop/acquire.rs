//! Deterministic hostile-target acquisition shared by auto-engaging behaviors.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::{astar, nav_pos::NavPos};

use crate::{
    components::{
        health::HealthComponent,
        location::{LocationComponent, LocationStaticData},
        owner::{self, OwnerComponent},
    },
    entity_index::EntityIndex,
    map::Map,
    session::GameSession,
    simulation_id::SimulationId,
};

/// Ticks between acquisition scans for one entity. Scans are staggered by
/// entity id (see [`due`]) so the load spreads across ticks.
pub const ACQUIRE_PERIOD: u32 = 8;

/// Ticks a recorded hit keeps steering acquisition: within this window an
/// attacker is preferred over nearer candidates, older stamps no longer aggro.
pub const HIT_MEMORY: u32 = 40;

/// Returns `true` on the ticks `id` is due for an acquisition scan.
pub fn due(id: SimulationId, tick: u32) -> bool {
    tick.wrapping_add(id.0).is_multiple_of(ACQUIRE_PERIOD)
}

/// Finds the target `seeker` should engage within `range` grid cells, if any.
///
/// A qualifying [`fresh_attacker`] wins; otherwise the nearest hostile,
/// damageable, interactable entity is chosen — nearest by
/// [`astar::rect_distance`], the same footprint measure the range gate uses —
/// with distance ties resolved to the lower [`SimulationId`], fully
/// deterministic.
pub fn find_target(world: &World, seeker: Entity, range: u32) -> Option<SimulationId> {
    if let Some(attacker) = fresh_attacker(world, seeker)
        && qualifies(world, seeker, attacker, range)
    {
        return Some(attacker);
    }

    let from = NavPos::from(position_of(world, seeker));
    let mut best: Option<(u32, SimulationId)> = None;
    for (id, _) in world.resource::<EntityIndex>().alive_entries() {
        if !qualifies(world, seeker, id, range) {
            continue;
        }
        let distance = footprint_distance(world, from, id);
        if best.is_none_or(|(best_distance, best_id)| {
            distance < best_distance || (distance == best_distance && id < best_id)
        }) {
            best = Some((distance, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Whether `target_id` is something `seeker` may auto-engage within `range`:
/// interactable, hostile, damageable, and with its footprint in range.
pub(super) fn qualifies(
    world: &World,
    seeker: Entity,
    target_id: SimulationId,
    range: u32,
) -> bool {
    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return false;
    };
    if target == seeker {
        return false;
    }

    let target_ref = world.entity(target);
    if !target_ref.contains::<HealthComponent>() {
        return false;
    }
    if !owner::are_hostile(
        world.resource::<GameSession>(),
        world.entity(seeker).get::<OwnerComponent>(),
        target_ref.get::<OwnerComponent>(),
    ) {
        return false;
    }

    let size = target_ref.get::<LocationStaticData>().unwrap().size();
    astar::in_range_of_rect(
        world.resource::<Map>().projection(),
        NavPos::from(position_of(world, seeker)),
        NavPos::from(target_ref.get::<LocationComponent>().unwrap().position),
        size,
        range,
    )
}

fn position_of(world: &World, entity: Entity) -> FixedUVec2 {
    world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position
}

/// The entity's most recent attacker, while the hit is still fresh (see
/// [`HIT_MEMORY`]).
pub(super) fn fresh_attacker(world: &World, entity: Entity) -> Option<SimulationId> {
    let tick = world.resource::<GameSession>().tick();
    world
        .entity(entity)
        .get::<HealthComponent>()
        .and_then(|health| health.last_hit())
        .filter(|hit| hit.tick + HIT_MEMORY >= tick)
        .map(|hit| hit.attacker)
}

/// Distance from `from` to the footprint of the alive entity with the given id.
fn footprint_distance(world: &World, from: NavPos, id: SimulationId) -> u32 {
    let entity = world.resource::<EntityIndex>().alive(id).unwrap();
    let entity_ref = world.entity(entity);
    astar::rect_distance(
        world.resource::<Map>().projection(),
        from,
        NavPos::from(entity_ref.get::<LocationComponent>().unwrap().position),
        entity_ref.get::<LocationStaticData>().unwrap().size(),
    )
}
