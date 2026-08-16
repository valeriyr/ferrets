//! Flee response: fleeing-stance entities run from whatever damaged them.

use bevy_ecs::world::World;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::{
    components::{
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        stance::StanceComponent,
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
/// The response is an idle entity's own initiative and never overrides a
/// command: whatever the player ordered outranks self-preservation, so a
/// transport flown into fire keeps flying where it was told and a commanded
/// worker keeps at its job. An idle entity that is hit takes a move directly
/// away from the attacker; a fresh hit once that run ends re-triggers the
/// response, so a pursued idler keeps making distance. The stamp's tick
/// keeps a single hit from triggering twice.
pub fn tick(world: &mut World) {
    let current_tick = world.resource::<GameSession>().tick();

    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if !entity_ref
            .get::<StanceComponent>()
            .is_some_and(|stance| stance.0.flees())
        {
            continue;
        }
        if !entity_def::of(world, entity).can_move() {
            continue;
        }
        // Commanded entities stay commanded: only an empty queue flees.
        if entity_ref
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| !queue.0.is_empty())
        {
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
            queue.push(
                Order::Move {
                    target,
                    size: CellSize::ONE,
                    range: 0,
                },
                Some(CancelPolicy::Soft),
            );
        }
    }
}

/// The cell [`FLEE_DISTANCE`] away from the attacker per axis, clamped to the
/// map. Axis signs keep the direction deterministic without normalization; on
/// an axis the two share (no direction to flee) the entity steps toward higher
/// coordinates.
fn flee_target(world: &World, from: FixedUVec2, attacker: FixedUVec2) -> FixedUVec2 {
    let from_cell = CellPos::from(from);
    let attacker_cell = CellPos::from(attacker);
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
