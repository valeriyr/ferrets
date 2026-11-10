//! Flee response: fleeing-stance entities run from whatever damaged them.

use bevy_ecs::world::World;
use ferrets_geometry::cell_pos::CellPos;

use super::orders;
use crate::{
    components::{
        last_hit::LastHitComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        stance::StanceComponent,
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    map::Map,
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
        if !entity_def::may_close(world, entity) {
            continue;
        }
        // One off the map has nowhere to run, and a frozen one does not.
        if entity_def::hidden(world, entity) {
            continue;
        }
        match entity_def::operation(world, entity) {
            Operation::Operating => {}
            Operation::Rising | Operation::Disabled(_) => continue,
            Operation::Stranded => unreachable!("a stranded body is hidden"),
        }
        // Commanded entities stay commanded: only an empty queue flees.
        if entity_ref
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| !queue.0.is_empty())
        {
            continue;
        }
        let Some(hit) = entity_ref.get::<LastHitComponent>() else {
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

        let anchor = entity_def::location(world, entity).anchor();
        let attacker_anchor = entity_def::whereabouts_rect(world, attacker).origin;
        let walk =
            orders::move_onto_cell(world, entity, flee_target(world, anchor, attacker_anchor));

        if let Some(mut queue) = world.entity_mut(entity).get_mut::<OrderQueueComponent>() {
            queue.push(walk, Some(CancelPolicy::Soft));
        }
    }
}

/// The cell [`FLEE_DISTANCE`] away from the attacker per axis, clamped to the
/// map. Axis signs keep the direction deterministic without normalization; on
/// an axis the two share (no direction to flee) the entity steps toward higher
/// coordinates.
fn flee_target(world: &World, from_cell: CellPos, attacker_cell: CellPos) -> CellPos {
    let grid = world.resource::<Map>().nav_grid();
    let (width, height) = (grid.width(), grid.height());

    let step = |own: u32, theirs: u32, limit: u32| -> u32 {
        if own >= theirs {
            own.saturating_add(FLEE_DISTANCE).min(limit - 1)
        } else {
            own.saturating_sub(FLEE_DISTANCE)
        }
    };
    CellPos::new(
        step(from_cell.x, attacker_cell.x, width),
        step(from_cell.y, attacker_cell.y, height),
    )
}
