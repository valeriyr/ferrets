//! Flee response: fleeing-stance entities run from whatever damaged them.

use bevy_ecs::world::World;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::nav_pos::NavPos;

use crate::{
    components::{
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        stance::{Stance, StanceComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    session::GameSession,
};

/// How far a fleeing entity runs from its attacker, in grid cells.
const FLEE_DISTANCE: u32 = 6;

/// Sends fleeing-stance entities running from hits that landed last tick.
///
/// The response is a flushed move directly away from the attacker — the one
/// deliberate interruption of in-progress orders, since a harvesting worker
/// must drop what it is doing. A fresh hit while running re-triggers the
/// response, so a pursued entity keeps running; the stamp's tick keeps a
/// single hit from triggering twice.
pub fn tick(world: &mut World) {
    let current_tick = world.resource::<GameSession>().tick();

    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if !matches!(
            entity_ref.get::<StanceComponent>(),
            Some(StanceComponent(Stance::Flee))
        ) {
            continue;
        }
        if !entity_def::of(world, entity).can_move() {
            continue;
        }
        let Some(hit) = entity_ref
            .get::<HealthComponent>()
            .and_then(|health| health.last_hit())
        else {
            continue;
        };
        // Hits land during the previous tick's order processing; older stamps
        // have already been answered.
        if hit.tick + 1 != current_tick {
            continue;
        }
        // An attacker already gone poses no threat to run from.
        let Some(attacker) = world.resource::<EntityIndex>().alive(hit.attacker) else {
            continue;
        };

        let position = entity_ref.get::<LocationComponent>().unwrap().position;
        let attacker_position = world
            .entity(attacker)
            .get::<LocationComponent>()
            .unwrap()
            .position;
        let target = flee_target(world, position, attacker_position);

        if let Some(mut queue) = world.entity_mut(entity).get_mut::<OrderQueueComponent>() {
            queue.push(Order::Move { target, range: 0 }, Some(CancelPolicy::Soft));
        }
    }
}

/// The cell [`FLEE_DISTANCE`] away from the attacker per axis, clamped to the
/// map. Axis signs keep the direction deterministic without normalization; on
/// an axis the two share (no direction to flee) the entity steps toward higher
/// coordinates.
fn flee_target(world: &World, from: FixedUVec2, attacker: FixedUVec2) -> FixedUVec2 {
    let from_cell = NavPos::from(from);
    let attacker_cell = NavPos::from(attacker);
    let grid = world.resource::<Map>().nav_grid();
    let (width, height) = (grid.width(), grid.height());

    let step = |own: u32, theirs: u32, limit: u32| -> u32 {
        if own >= theirs {
            own.saturating_add(FLEE_DISTANCE).min(limit - 1)
        } else {
            own.saturating_sub(FLEE_DISTANCE)
        }
    };
    FixedUVec2::new(
        FixedU64::from_num(step(from_cell.x, attacker_cell.x, width)),
        FixedU64::from_num(step(from_cell.y, attacker_cell.y, height)),
    )
}
