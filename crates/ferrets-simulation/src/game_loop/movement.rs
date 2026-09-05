//! Move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeMap;

use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use ferrets_content::{entity_stats::EntityStatId, location::Solidity};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{
    FixedI64, FixedU64, facing::Facing, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2,
};
use ferrets_pathfinder::{
    astar,
    hierarchy::ClusterPos,
    hpa::{self, Crossing, PathShape, PlanTarget},
    layer_mask::LayerMask,
    mover_shape::MoverShape,
};
use ferrets_physics::{body, terrain};

use super::{
    orders::{self, Processing, Refusal},
    turn,
};
use crate::{
    components::{
        hidden::HiddenComponent,
        location::LocationComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    session::GameSession,
};

/// Ticks to wait for a blocked cell to clear before recalculating the path.
const BLOCKED_WAIT_TICKS: u32 = 5;

/// How many cells ahead a local detour rejoins its segment.
const DETOUR_LOOKAHEAD: usize = 6;

/// How far the acceptance range grows as a mover keeps failing to close in,
/// so a crowded destination settles into a ring instead of a grinding queue.
const MAX_ACCEPTANCE_GROWTH: u32 = 4;

/// Blockage escalations a walk may burn before giving up where it stands.
const GIVE_UP_ESCALATIONS: u32 = 8;

/// How near an intermediate waypoint counts as reaching it: half a body, so a
/// mover the pushing pass keeps nudging is not held to an exact hit it may never
/// land, and the next waypoint pulls from far ahead anyway.
const WAYPOINT_POP_SLACK: FixedU64 = FixedU64::lit("0.5");

/// The share of a tick's walk below which a slid step counts as a crawl rather
/// than progress. An exact binary fraction, so scaling a speed by it is exact.
const CRAWL_SHARE: FixedU64 = FixedU64::lit("0.125");

/// The share of a tick's walk a step must keep for its direction to be the
/// body's heading. Larger than [`CRAWL_SHARE`]: a crawl is still real travel and
/// the stall clock has to judge it, while a look wants only the steps the eye
/// can follow.
const HEADING_SHARE: FixedU64 = FixedU64::lit("0.25");

/// One shared plan's identity. The mover's shape is part of it because each
/// shape has its own abstraction: a corridor planned for one clearance means
/// nothing to another, and their independently-minted region numbers are not
/// comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ShareKey {
    /// The shape the plan routes.
    shape: MoverShape,
    /// The goal footprint's anchor cell.
    goal: CellPos,
    /// The goal footprint's size.
    goal_size: CellSize,
    /// The stop distance the plan accepts.
    stop_distance: u32,
    /// The cluster the plan starts from.
    cluster: ClusterPos,
    /// The start's connectivity region under the shape's abstraction.
    region: u32,
}

/// One tick's shared movement plans: a fanned group order plans its abstract
/// corridor once per start cluster, and every unit refines its own cells
/// from it.
#[derive(Resource, Debug, Default)]
pub struct MovePlanShare {
    /// The tick the stored plans belong to; any other tick reads as empty.
    tick: u32,
    /// The tick's plans by identity.
    plans: BTreeMap<ShareKey, (PlanTarget, Vec<Crossing>)>,
}

impl MovePlanShare {
    /// The plans of `tick`, dropping any staler ones.
    fn for_tick(&mut self, tick: u32) -> &mut BTreeMap<ShareKey, (PlanTarget, Vec<Crossing>)> {
        if self.tick != tick {
            self.tick = tick;
            self.plans.clear();
        }
        &mut self.plans
    }
}

/// Whether `entity` may start a Move: its type moves and it operates.
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    if !entity_def::of(world, entity).can_move() {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)
}

/// Called once when a Move order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`]. Whether the
/// target is already reached is decided in [`process`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    insert_driver(entity, world);
    OrderState::InProcessing
}

/// Called for every Move entry that has a cancel policy.
///
/// Returns the new state:
/// - `Finished` — stop immediately; the driver component is removed.
/// - `InProcessing` — Soft cancel while mid-crossing: path trimmed to the current
///   target so the entity completes only the immediate crossing then stops.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    policy: CancelPolicy,
    entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    match policy {
        CancelPolicy::Force => {
            // A cell-model walk cut down mid-crossing holds the crossing's
            // target cell — its departed cell was released when the
            // crossing began. Snap onto the claimed cell, so whoever frees
            // the footprint next (death, hiding) releases the cell the unit
            // actually holds rather than a neighbor somebody else may have
            // claimed since.
            if let Some(movement) = world.entity(entity).get::<MoveComponent>() {
                let claimed = movement.path.last().copied();
                let position = world
                    .entity(entity)
                    .get::<LocationComponent>()
                    .map(|location| location.position);
                if let (Some(claimed), Some(position)) = (claimed, position)
                    && let MovementModel::Cell = world.resource::<Map>().movement_model()
                    && is_mid_crossing(position)
                {
                    world
                        .entity_mut(entity)
                        .get_mut::<LocationComponent>()
                        .unwrap()
                        .position = FixedUVec2::from(claimed);
                }
            }
            world.entity_mut(entity).remove::<MoveComponent>();
            OrderState::Finished
        }
        CancelPolicy::Soft => {
            if entry_state == OrderState::InProcessing
                && let Some(mut move_component) =
                    world.entity_mut(entity).get_mut::<MoveComponent>()
            {
                move_component.leave_only_current_target();
                return OrderState::InProcessing;
            }
            // Order has not started yet — nothing to finish, discard immediately.
            OrderState::Finished
        }
    }
}

/// Advance a Move order by one tick under the session's movement model.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    Processing::state(match world.resource::<Map>().movement_model() {
        MovementModel::Cell => process_cell(entity, order, world),
        MovementModel::Continuous => process_continuous(entity, order, world),
    })
}

/// How many ticks without progress count as one blockage escalation for a
/// continuous mover being crowded off its way.
const CONTINUOUS_STUCK_TICKS: u32 = 15;

/// Advance a Move order by one tick under [`MovementModel::Continuous`].
///
/// The mover walks its planned waypoints as a free position: no crossings,
/// no blocked checks, no claim bookkeeping — contact is the pushing pass's
/// concern, and it also rebuilds the claim plane from the settled bodies
/// each tick.
fn process_continuous(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let (target, size, range) = order.move_params().expect("Move order must have params");

    let position = entity_def::position(world, entity);
    let speed = entity_def::effective_stat(world, entity, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let location_def = entity_def::of(world, entity).location.unwrap();
    let shape = MoverShape::new(location_def.occupation(), location_def.size());

    let Some(mut move_component) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    // Crowded off the way: escalations grow the acceptance and bound the
    // grind, like the cell ladder's. Progress means a new record distance
    // to the current waypoint — not to the goal, which a legitimate detour
    // around a lake walks AWAY from for its whole far leg — and motion
    // alone proves nothing, since a body churned around a crowd crosses
    // cells without getting anywhere. Records are strictly decreasing and
    // reset whenever a waypoint is consumed, so a walk that stops closing
    // in escalates within a bounded number of ticks however it is shoved
    // around, while a long way round never does.
    // The mover is judged by its rounded anchor — the cell its claim is
    // stamped on. The position's floor sits one cell short whenever a push
    // pressed the body against what it walks toward.
    let current_cell = body::anchor(position);
    let projection = world.resource::<Map>().projection();
    let pursued = move_component
        .path
        .last()
        .map_or(target, |&waypoint| FixedUVec2::from(waypoint));
    let distance = position.distance(pursued);
    if distance < move_component.best_distance {
        move_component.record_progress(distance);
    } else {
        move_component.wait_ticks += 1;
        if move_component.wait_ticks >= CONTINUOUS_STUCK_TICKS {
            if move_component.escalate() > GIVE_UP_ESCALATIONS {
                return OrderState::Finished;
            }

            // Peer arrival, as on the cell ladder's rung: a settled ally
            // already inside the acceptance means the spot is taken —
            // arriving beside it kills the pushing contest. Point moves
            // only; a ranged walk finishing short would break its parent
            // order's range semantics.
            let goal = CellRect::new(CellPos::from(target), size);
            let effective_range = acceptance(range, move_component.frustration);
            if range == 0
                && projection.in_range_of_rect(current_cell, goal, effective_range + 1)
                && resting_ally_within(world, entity, shape.mask, goal, effective_range)
            {
                return OrderState::Finished;
            }

            // Detour, as on the cell ladder's rung: splice a claim-aware
            // local path around the crowd toward the current waypoint. The
            // claim plane mirrors the standing bodies, and claims cost
            // instead of block, so the mover's own straddled cells cannot
            // wall it in.
            if !move_component.detoured
                && let Some(&waypoint) = move_component.path.last()
            {
                let cells = {
                    let map = world.resource::<Map>();
                    hpa::detour(
                        map.nav_grid(),
                        map.hierarchy(),
                        projection,
                        current_cell,
                        shape,
                        waypoint,
                    )
                };
                if let Some(cells) = cells
                    && !cells.is_empty()
                {
                    move_component.splice_detour(cells);
                }
            }
        }
    }
    let effective_range = acceptance(range, move_component.frustration);
    // An uncrowded point order lands exactly on the ordered spot, to the
    // bit — a lattice target is just a spot that happens to be a cell
    // origin. Contested spots do not grind forever: the stall clock raises
    // frustration, and a frustrated walk accepts by cells like every ranged
    // or wider-than-one-cell move.
    let arrived = if range == 0 && size == CellSize::ONE && move_component.frustration == 0 {
        position == target
    } else {
        // One check serves both contracts, because the two distances differ
        // on purpose: the goal expands by the mover's footprint only for a
        // ranged order (`accepted_by` is the identity at range 0, keeping a
        // point destination an anchor contract), while the measured distance
        // is the effective acceptance — the order's own range for a ranged
        // walk, the frustration ring for a crowded point move.
        projection.in_range_of_rect(
            current_cell,
            CellRect::new(CellPos::from(target), size).accepted_by(shape.size, range),
            effective_range,
        )
    };
    if arrived {
        return OrderState::Finished;
    }

    // A point order is honored to the bit, so its walk cannot end on the
    // goal cell's origin like the cell-resolution plans do: the final leg
    // aims at the ordered spot itself.
    let point_order = range == 0 && size == CellSize::ONE;

    if move_component.path.is_empty() {
        // Wound down: the step it was told to finish has landed, so the walk is
        // over. Asking for the next segment here would replan the whole journey
        // the cancel just discarded.
        if move_component.winding_down {
            return OrderState::Finished;
        }
        if point_order && current_cell == CellPos::from(target) {
            // Already standing on the goal cell — a plan would be empty;
            // walk the last sub-cell stretch to the spot directly.
            move_component.pursue(CellPos::from(target));
        } else {
            match next_segment(
                world,
                &mut move_component,
                shape,
                location_def.solidity(),
                position,
                target,
                size,
                range,
            ) {
                None => return OrderState::Finished,
                Some(segment) if segment.is_empty() => return OrderState::Finished,
                Some(segment) => move_component.pursue_segment(segment),
            }
        }
    }

    // Step toward the waypoint, but never overlap the body onto a
    // statically blocked cell: a push may have carried the position off the
    // planned cells, and the straight line back would cut corners through
    // footprints. A blocked axis is dropped; a fully blocked step throws
    // the segment away at once — the wall is terrain, and unlike a crowd it
    // never clears, so the next tick replans from where the body actually
    // stands instead of waiting out the crowd-patience clock.
    let final_waypoint = move_component.path.len() == 1 && move_component.corridor.is_empty();
    let waypoint = if final_waypoint
        && point_order
        && move_component
            .plan
            .is_none_or(|plan| plan.cell == CellPos::from(target))
        && *move_component.path.last().unwrap() == CellPos::from(target)
    {
        target
    } else {
        FixedUVec2::from(*move_component.path.last().unwrap())
    };
    let desired = projection.step_toward(position, waypoint, speed);
    let radius = entity_def::radius(world, entity);
    let slide = |aim: FixedUVec2, world: &World| {
        terrain::slide_toward(
            world.resource::<Map>().nav_grid(),
            shape.mask,
            position,
            shape.size,
            aim,
            radius,
        )
    };
    let mut new_pos = slide(desired, world);
    // Aiming into a wall rather than along it: the slide keeps the step's
    // dominant axis where it can, so losing that one and keeping the minor one
    // means the wall is across the way, not beside it.
    let aimed_into_wall = if desired.x.abs_diff(position.x) >= desired.y.abs_diff(position.y) {
        desired.x != position.x && new_pos.x == position.x
    } else {
        desired.y != position.y && new_pos.y == position.y
    };
    // Walked along rather than grazed: what is left of a step aimed past a wall
    // is the minor axis's share of it — a fraction of the walk — so the tick is
    // better spent on the way that is open, which is also what clears the block.
    //
    // Decided afresh every tick on purpose: a body held to one axis until it
    // lines up walks itself into the corner, where both axes are shut and the
    // walk stops dead for a tick. Feeling its way round costs a couple of turns
    // and keeps moving.
    let walking_along_wall = aimed_into_wall && {
        // A tick's walk along one axis, stopping at the waypoint's own
        // coordinate so the walk cannot overshoot what it is walking to. Spelled
        // out rather than taken from [`Projection::step_toward`]: the slide takes
        // a destination, so the aim has to carry the speed limit itself, and one
        // axis at full rate is exactly `speed` — which the projection's own
        // scaling would only round its way back to.
        let along = |from: FixedU64, toward: FixedU64| {
            if toward >= from {
                from.saturating_add(speed).min(toward)
            } else {
                from.saturating_sub(speed).max(toward)
            }
        };
        // The axis that did not move is the shut one, whichever was dominant, so
        // the other is the way that is open. Neither moving names the wrong one
        // and costs nothing: the aim comes out as the body's own position, which
        // the slide cannot better.
        let aim = if new_pos.x == position.x {
            FixedUVec2::new(position.x, along(position.y, waypoint.y))
        } else {
            FixedUVec2::new(along(position.x, waypoint.x), position.y)
        };
        let alongside = slide(aim, world);
        // A step newly walled gives way only to a better one, so a wall that
        // costs the body nothing is not walked along at all.
        let further = position.distance(alongside) > position.distance(new_pos);
        if further {
            new_pos = alongside;
        }
        further
    };
    let travelled = position.distance(new_pos);
    // A slide that dropped an axis and kept next to nothing of the step is a
    // body pressed diagonally into a footprint corner: the free axis converges
    // on the corner's tangent line without ever clearing it, and the crumbs it
    // yields keep reading as progress, so the stall clock never fires either.
    // Pressed and crawling is walled, just slower about it.
    let pressed_crawl = new_pos != desired && travelled < speed * CRAWL_SHARE;
    if (new_pos == position || pressed_crawl) && desired != position {
        // Walled off: a push pressed the body against a footprint, and from
        // this off-lattice spot the straight line to the waypoint clips the
        // corner. Regain the cell's own lattice point — always reachable,
        // since pulling the circle inward only shrinks what it overlaps —
        // then resume the same waypoint, through a claim-aware detour when
        // one exists: the geometry that walled the body off once will wall
        // it off again from the same lattice point, and the pinning crowd
        // shows up in the claim plane, not the static one. The lattice point
        // is held as a real waypoint rather than stepped toward once: a
        // single step can land short, and a replan from short used to walk
        // straight back into the corner — a full-speed ping-pong. Each
        // regain escalates, because the loop it breaks yields a fresh
        // record every pass and never trips the stall clock on its own.
        if move_component.escalate() > GIVE_UP_ESCALATIONS {
            return OrderState::Finished;
        }
        let detour = match move_component.path.last() {
            Some(&waypoint) if !move_component.detoured => {
                let map = world.resource::<Map>();
                hpa::detour(
                    map.nav_grid(),
                    map.hierarchy(),
                    projection,
                    current_cell,
                    shape,
                    waypoint,
                )
            }
            _ => None,
        };
        if let Some(cells) = detour
            && !cells.is_empty()
        {
            move_component.splice_detour(cells);
        }
        move_component.regain(current_cell);
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }
    // The step taken, not the one aimed for: a slide along terrain keeps only
    // what the way allows of the aim, and a body that walks along a bank while
    // looking into it is a body looking where it is not going.
    //
    // Only a step worth seeing is a heading. A crumb — a lattice point being
    // regained, a scrape past a corner — moves the body a pixel or two along
    // some axis it does not mean to travel, and turning down it spins the body
    // over a movement nobody can see. Keeping the previous look is right for a
    // step that landed nowhere too, which has no direction to offer at all.
    if travelled >= speed * HEADING_SHARE
        && let Some(wanted) = Facing::of(FixedVec2::new(
            new_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
            new_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
        ))
    {
        // Decided on the step the body would really take rather than the one it
        // aimed for, so a walk along a bank is judged by the bank: gating on the
        // aim would stand the body up to face a wall, walk it along, and stand it
        // up again every tick it ran beside one.
        move_component.pivoting = turn::pivoting(world, entity, wanted, move_component.pivoting);
        if move_component.pivoting {
            turn::toward(world, entity, wanted, turn::Rate::Standing);
            // The tick went on coming round, so it is not a tick of getting
            // nowhere — see `MoveComponent::excuse`.
            move_component.excuse();
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        }
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    world
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = new_pos;
    // Touching the ordered spot completes a point order on the spot, within
    // the same tick: waiting for the next tick's arrival check would let
    // the pushing pass shove the body off first, and a contested point
    // would be touched, lost, and re-walked forever.
    if point_order && new_pos == target {
        return OrderState::Finished;
    }
    // A wall the slide could not be walked along: the way round the corner may
    // still be a clear line somewhere else, which only a search finds. Nothing
    // has gone wrong yet, so this only re-aims.
    if aimed_into_wall
        && !walking_along_wall
        && !move_component.detoured
        && new_pos != position
        && let Some(&pursued) = move_component.path.last()
    {
        let cells = {
            let map = world.resource::<Map>();
            hpa::detour(
                map.nav_grid(),
                map.hierarchy(),
                projection,
                current_cell,
                shape,
                pursued,
            )
        };
        // A fruitless search must not be retried every tick: `splice_detour`
        // is what normally marks the blockage's one detour spent, so a search
        // that found nothing marks it here instead. The next escalation clears
        // the mark and tries again.
        let Some(cells) = cells.filter(|cells| !cells.is_empty()) else {
            move_component.detoured = true;
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        };
        move_component.splice_detour(cells);
        // The splice replaced the pursued waypoint, so the arrival test below
        // has nothing left to say about it — comparing against the stale one
        // would consume the detour's own first cell, the very step that rounds
        // the corner. This tick's step is already committed; the walk resumes
        // against the detour next tick.
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }

    // Intermediate waypoints pop within half a body of slack (see
    // [`WAYPOINT_POP_SLACK`]). The final one pops only on the exact spot, so
    // arrival stays precise — and so does a regained lattice point, whose whole
    // purpose is standing exactly on it.
    let reached = if final_waypoint || move_component.regaining {
        new_pos == waypoint
    } else {
        new_pos.distance(waypoint) <= WAYPOINT_POP_SLACK
    };
    if reached {
        move_component.consume_waypoint();
    }
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// Advance a Move order by one tick under [`MovementModel::Cell`].
///
/// Each tick:
/// 1. If mid-crossing (position is not on a cell origin), continue moving toward
///    `path.last()`, popping the waypoint on arrival.
/// 2. Otherwise finish if the goal is within the order's stop distance, or
///    calculate a path if needed, claim the next cell in the nav grid, and begin
///    the crossing.
///
/// An emptied `path` is a spent corridor leg, not an arrival — only the goal
/// check in step 2 ends the order, so a walk planned as a corridor asks for its
/// next leg instead of stopping at the first crossing.
///
/// A crossing's cells are claimed when the entity **starts** crossing into
/// them, not on arrival. `path.last()` is the active crossing target; it is popped
/// only when the entity arrives, so no separate current-target field is needed.
///
/// `MoveComponent` is taken from the entity at the start and reinserted on
/// `InProcessing`; on `Finished` it is simply dropped (removed by not reinserting).
fn process_cell(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let (target, size, range) = order.move_params().expect("Move order must have params");

    let position = entity_def::position(world, entity);
    let speed = entity_def::effective_stat(world, entity, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let location_def = entity_def::of(world, entity).location.unwrap();
    let shape = MoverShape::new(location_def.occupation(), location_def.size());

    let Some(mut move_component) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    let projection = world.resource::<Map>().projection();

    // Mid-crossing: position is between the crossing's two cell origins.
    if is_mid_crossing(position) {
        debug_assert_on_crossing(
            position,
            move_component.moving_from,
            *move_component.path.last().unwrap(),
        );
        let target_pos = FixedUVec2::from(*move_component.path.last().unwrap());
        let new_pos = projection.step_toward(position, target_pos, speed);
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .unwrap()
            .position = new_pos;

        if new_pos == target_pos {
            move_component.path.pop();
            // An emptied path is a spent corridor leg, not necessarily an
            // arrival: a corridor is refined one leg at a time, so the goal
            // decides. Judging it here rather than leaving it to the next
            // tick's at-rest check keeps a walk that really has arrived
            // finishing on the tick it lands.
            if move_component.path.is_empty()
                && cell_walk_done(
                    &move_component,
                    projection,
                    CellPos::from(new_pos),
                    shape,
                    target,
                    size,
                    range,
                )
            {
                return OrderState::Finished;
            }
        }
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }

    // At rest on a cell — check if the goal is already reached. Frustration
    // grows the acceptance of point moves so a crowded destination settles
    // into a ring; ranged walks keep their exact contract — a chase that
    // finished short would break its parent order's range semantics.
    if cell_arrived(
        projection,
        CellPos::from(position),
        shape,
        target,
        size,
        range,
        move_component.frustration,
    ) {
        return OrderState::Finished;
    }
    // The crowd ladder measures against the same acceptance the arrival check
    // just used.
    let effective_range = acceptance(range, move_component.frustration);

    // Continue the plan — or make one — when the current segment runs out.
    if move_component.path.is_empty() {
        match next_segment(
            world,
            &mut move_component,
            shape,
            location_def.solidity(),
            position,
            target,
            size,
            range,
        ) {
            None => return OrderState::Finished,
            Some(segment) if segment.is_empty() => return OrderState::Finished,
            Some(segment) => {
                // Store reversed so path.last() = next immediate step (pop from back).
                move_component.path = segment.into_iter().rev().collect();
            }
        }
    }

    // Start a crossing into the next cell.
    let next_cell = *move_component.path.last().unwrap();

    if !world.resource::<Map>().can_step_footprint(
        shape.mask,
        shape.size,
        CellPos::from(position),
        next_cell,
    ) {
        return resolve_blocked_crossing(
            entity,
            world,
            move_component,
            position,
            speed,
            shape,
            target,
            size,
            range,
            effective_range,
        );
    }
    move_component.forgive();

    let heading = Facing::of(FixedVec2::new(
        FixedUVec2::from(next_cell).x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        FixedUVec2::from(next_cell).y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
    ));
    // A body that must plant its feet to come round does so before it stakes a
    // claim on the cell it is crossing into: a claim is the crossing, and one
    // held while the body is still turning would reserve ground it has not set
    // off for.
    if let Some(wanted) = heading {
        move_component.pivoting = turn::pivoting(world, entity, wanted, move_component.pivoting);
        if move_component.pivoting {
            turn::toward(world, entity, wanted, turn::Rate::Standing);
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        }
    }

    // Claim the footprint entered and release the one left behind, before any
    // position change. Claims live on the grid's unit plane, invisible to the
    // hierarchy. Passable entities never claim cells.
    let current_cell = CellPos::from(position);
    if location_def.solidity().claims_cells() {
        world
            .resource_mut::<Map>()
            .step_claim(shape.mask, shape.size, current_cell, next_cell);
    }

    move_component.moving_from = current_cell;

    let next_pos = FixedUVec2::from(next_cell);
    let new_pos = projection.step_toward(position, next_pos, speed);

    if let Some(wanted) = heading {
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    world
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = new_pos;

    if new_pos == next_pos {
        move_component.path.pop();
        // As above: the spent leg ends the walk only once the goal is met.
        if move_component.path.is_empty()
            && cell_walk_done(
                &move_component,
                projection,
                CellPos::from(new_pos),
                shape,
                target,
                size,
                range,
            )
        {
            return OrderState::Finished;
        }
    }

    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// The next run of cells to walk: the continuation of the current plan, or a
/// fresh one — hierarchical when the map serves the mover's mask, flat
/// otherwise. `None` and an empty segment both mean the entity is as close
/// as it can get.
#[allow(clippy::too_many_arguments)]
fn next_segment(
    world: &mut World,
    move_component: &mut MoveComponent,
    shape: MoverShape,
    solidity: Solidity,
    position: FixedUVec2,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
) -> Option<Vec<CellPos>> {
    let from = body::anchor(position);
    let path_shape = match world.resource::<Map>().movement_model() {
        MovementModel::Cell => PathShape::CellSteps,
        MovementModel::Continuous => PathShape::Waypoints,
    };

    // Continue the standing plan.
    if let Some(plan) = move_component.plan {
        let map = world.resource::<Map>();
        let projection = map.projection();
        if move_component.corridor.is_empty()
            && projection.in_reach(from, shape.size, plan.rect(), plan.stop)
        {
            // As close as the plan gets; the caller already checked the
            // order's own goal and found it unsatisfied.
            return None;
        }
        let refined = hpa::refine(
            map.nav_grid(),
            map.hierarchy(),
            projection,
            from,
            shape,
            &mut move_component.corridor,
            plan,
            path_shape,
        );
        match refined {
            Some(segment) => return Some(segment),
            None => {
                // The map changed under the plan — replan from scratch.
                move_component.corridor.clear();
                move_component.plan = None;
            }
        }
    }

    // A blocked crossing asked for a claim-aware route around the blocker;
    // the flat search is also all a map without an abstraction for this
    // mask offers.
    let (projection, serves, region) = {
        let map = world.resource::<Map>();
        let region = if map.hierarchy().serves(shape) {
            map.hierarchy().region_of(from, shape)
        } else {
            None
        };
        (map.projection(), map.hierarchy().serves(shape), region)
    };
    if move_component.avoid_claims || !serves {
        move_component.avoid_claims = false;
        // The mover's own footprint claim would wall this claim-honoring
        // search in at its start — a wide mover overlaps its own claim from
        // every neighboring anchor — so it is lifted for exactly this query
        // and put straight back. A passable mover claims nothing, so there
        // is nothing to lift; cells under its body may be someone else's
        // claim, and lifting those would route it through ground others hold.
        let own = if solidity.claims_cells() {
            world
                .resource_mut::<Map>()
                .take_claim(shape.mask, from, shape.size)
        } else {
            Vec::new()
        };
        let path = {
            let map = world.resource::<Map>();
            astar::find_path(
                map.nav_grid(),
                projection,
                position,
                shape,
                target,
                size,
                range,
            )
        };
        world.resource_mut::<Map>().restore_claim(shape.mask, &own);
        return path.map(|path| path.into_iter().map(CellPos::from).collect());
    }

    // A fresh plan: reuse the tick's shared corridor for this goal and start
    // locality, planning it once for the whole fanned group.
    let region = region?;
    let tick = world.resource::<GameSession>().tick();
    let key = ShareKey {
        shape,
        goal: CellPos::from(target),
        goal_size: size,
        stop_distance: range,
        cluster: world.resource::<Map>().hierarchy().cluster_of(from),
        region,
    };

    let shared = world
        .resource_mut::<MovePlanShare>()
        .for_tick(tick)
        .get(&key)
        .cloned();
    if let Some((plan_target, corridor)) = shared {
        move_component.corridor = corridor;
        move_component.plan = Some(plan_target);
        let map = world.resource::<Map>();
        let refined = hpa::refine(
            map.nav_grid(),
            map.hierarchy(),
            projection,
            from,
            shape,
            &mut move_component.corridor,
            plan_target,
            path_shape,
        );
        match refined {
            Some(segment) => return Some(segment),
            None => {
                // A straggler the shared corridor cannot serve plans its own.
                move_component.corridor.clear();
                move_component.plan = None;
            }
        }
    }

    let (plan_target, corridor) = {
        let map = world.resource::<Map>();
        hpa::plan_corridor(
            map.nav_grid(),
            map.hierarchy(),
            projection,
            from,
            shape,
            PlanTarget {
                cell: CellPos::from(target),
                size,
                stop: range,
            },
        )?
    };
    world
        .resource_mut::<MovePlanShare>()
        .for_tick(tick)
        .insert(key, (plan_target, corridor.clone()));

    move_component.corridor = corridor;
    move_component.plan = Some(plan_target);
    let map = world.resource::<Map>();
    hpa::refine(
        map.nav_grid(),
        map.hierarchy(),
        projection,
        from,
        shape,
        &mut move_component.corridor,
        plan_target,
        path_shape,
    )
}

/// The range a walk accepts right now: point moves grow with frustration so
/// crowds settle into a ring; ranged walks keep their exact contract.
fn acceptance(range: u32, frustration: u32) -> u32 {
    if range == 0 {
        frustration.min(MAX_ACCEPTANCE_GROWTH)
    } else {
        range
    }
}

/// Whether a cell-model walk whose step has just landed ends here: either the
/// goal is satisfied, or the walk is winding down and this was the step it was
/// told to finish.
fn cell_walk_done(
    move_component: &MoveComponent,
    projection: Projection,
    cell: CellPos,
    shape: MoverShape,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
) -> bool {
    move_component.winding_down
        || cell_arrived(
            projection,
            cell,
            shape,
            target,
            size,
            range,
            move_component.frustration,
        )
}

/// Whether a cell-model walk's goal is satisfied from `cell`.
///
/// The expansion is keyed on the order's own range while the distance uses the
/// effective acceptance; the two differ on purpose, for the reasons the arrival
/// check under the continuous model spells out.
fn cell_arrived(
    projection: Projection,
    cell: CellPos,
    shape: MoverShape,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
    frustration: u32,
) -> bool {
    projection.in_range_of_rect(
        cell,
        CellRect::new(CellPos::from(target), size).accepted_by(shape.size, range),
        acceptance(range, frustration),
    )
}

/// Works down the crowd ladder when the next cell of a crossing is held:
/// swap with a head-on counterpart, arrive beside a settled ally, skip a
/// waypoint an idle ally sits on, ask an idle ally to step aside, wait
/// briefly, splice a local detour, and finally throw the plan away for a
/// claim-aware repath — bounded by a give-up budget.
#[allow(clippy::too_many_arguments)]
fn resolve_blocked_crossing(
    entity: Entity,
    world: &mut World,
    mut move_component: MoveComponent,
    position: FixedUVec2,
    speed: FixedU64,
    shape: MoverShape,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
    effective_range: u32,
) -> OrderState {
    let current_cell = CellPos::from(position);
    let next_cell = *move_component.path.last().unwrap();
    let projection = world.resource::<Map>().projection();

    if let Some(blocker) = claimant_at(world, shape.mask, next_cell) {
        let blocker_position = entity_def::position(world, blocker);
        let blocker_at_rest = !is_mid_crossing(blocker_position);

        // Swap: the blocker rests on my next cell and wants mine — the
        // head-on case. Both crossings run at once; no claim bit changes
        // hands visibly, each side keeps exactly its own claim — which is
        // only an identity when both occupy the same layers **and the same
        // footprint**, so unequal shapes fall through to the other rungs:
        // exchanging a 1×1 claim for a 2×2 one would leave ghost-claimed
        // cells behind one body and unclaimed cells under the other.
        if blocker_at_rest
            && entity_def::of(world, blocker)
                .location
                .is_some_and(|location| {
                    location.occupation() == shape.mask && location.size() == shape.size
                })
            && world
                .entity(blocker)
                .get::<MoveComponent>()
                .is_some_and(|counter| counter.path.last() == Some(&current_cell))
        {
            return swap_crossings(
                entity,
                world,
                move_component,
                position,
                speed,
                projection,
                shape,
                current_cell,
                next_cell,
                blocker,
                target,
                size,
                range,
            );
        }

        let blocker_idle = world
            .entity(blocker)
            .get::<OrderQueueComponent>()
            .is_none_or(|queue| queue.0.is_empty());
        let blocker_moves = entity_def::of(world, blocker).can_move();

        if blocker_at_rest && blocker_idle && blocker_moves && allied(world, entity, blocker) {
            // Peer arrival: a settled ally already inside my acceptance
            // means the spot is taken — arriving beside it kills the
            // pushing contest. Point moves only; a ranged walk finishing
            // short would break its parent order's range semantics.
            if range == 0
                && projection.in_range_of_rect(
                    next_cell,
                    CellRect::new(CellPos::from(target), size),
                    effective_range,
                )
                && projection.in_range_of_rect(
                    current_cell,
                    CellRect::new(CellPos::from(target), size),
                    effective_range + 1,
                )
            {
                return OrderState::Finished;
            }

            // Waypoint skip: it sits exactly on an intermediate waypoint
            // and the following one is adjacent — cut the corner past it.
            if move_component.path.len() > 1 {
                let following = move_component.path[move_component.path.len() - 2];
                if current_cell.x.abs_diff(following.x) <= 1
                    && current_cell.y.abs_diff(following.y) <= 1
                {
                    move_component.path.pop();
                    move_component.wait_ticks = 0;
                    world.entity_mut(entity).insert(move_component);
                    return OrderState::InProcessing;
                }
            }

            // Yield: ask it to step out of the walk's way, then wait below
            // while it does. The pushed order is prepared here — the tick's
            // own prepare phase has already passed.
            if let Some(aside) = yield_target(
                world,
                blocker,
                CellRect::new(current_cell, shape.size),
                next_cell,
            ) {
                let mut queue = world
                    .entity_mut(blocker)
                    .take::<OrderQueueComponent>()
                    .expect("idle blockers keep their order queue");
                queue.push_front(Order::Move {
                    target: FixedUVec2::from(aside),
                    size: CellSize::new(1, 1),
                    range: 0,
                });
                super::orders::prepare_front(blocker, &mut queue, world);
                world.entity_mut(blocker).insert(queue);
            }
        }
    }

    // Wait: for a mover passing through, or a yielded ally stepping off.
    if move_component.wait_ticks < BLOCKED_WAIT_TICKS {
        move_component.wait_ticks += 1;
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }
    if move_component.escalate() > GIVE_UP_ESCALATIONS {
        return OrderState::Finished;
    }

    // Local detour: rejoin the segment a few cells ahead, claims as soft
    // costs, spliced in place of the blocked stretch.
    if !move_component.detoured {
        move_component.detoured = true;
        let rejoin_index = move_component
            .path
            .len()
            .saturating_sub(1 + DETOUR_LOOKAHEAD);
        let rejoin = move_component.path[rejoin_index];
        let detour = {
            let map = world.resource::<Map>();
            hpa::detour(
                map.nav_grid(),
                map.hierarchy(),
                projection,
                current_cell,
                shape,
                rejoin,
            )
        };
        if let Some(cells) = detour
            && !cells.is_empty()
        {
            move_component.path.truncate(rejoin_index);
            move_component.path.extend(cells.into_iter().rev());
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        }
    }

    // Full repath: throw the plan away; the next plan honors unit claims.
    move_component.repath_avoiding_claims();
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// Starts both crossings of a head-on swap: this entity toward `next_cell`,
/// the blocker toward `current_cell`. Claim bits stay untouched — the
/// exchange leaves each cell claimed by its new owner.
#[allow(clippy::too_many_arguments)]
fn swap_crossings(
    entity: Entity,
    world: &mut World,
    mut move_component: MoveComponent,
    position: FixedUVec2,
    speed: FixedU64,
    projection: Projection,
    shape: MoverShape,
    current_cell: CellPos,
    next_cell: CellPos,
    blocker: Entity,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
) -> OrderState {
    let next_pos = FixedUVec2::from(next_cell);
    let current_pos = FixedUVec2::from(current_cell);

    // The counterpart's crossing starts here; its own order processing
    // continues it as any other mid-crossing tick.
    let blocker_speed = entity_def::effective_stat(world, blocker, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let blocker_new = projection.step_toward(next_pos, current_pos, blocker_speed);
    {
        let mut blocker_mut = world.entity_mut(blocker);
        let mut counter_move = blocker_mut.get_mut::<MoveComponent>().unwrap();
        counter_move.moving_from = next_cell;
        counter_move.forgive();
        if blocker_new == current_pos {
            counter_move.path.pop();
        }
        let mut blocker_location = blocker_mut.get_mut::<LocationComponent>().unwrap();
        blocker_location.position = blocker_new;
    }
    // Both halves of a swap are crossings, so both come round as they walk.
    // Neither holds still first: the two have already agreed to trade places,
    // and a body standing to turn would leave the other walking into it.
    if let Some(wanted) = Facing::of(FixedVec2::new(
        current_pos.x.to_num::<FixedI64>() - next_pos.x.to_num::<FixedI64>(),
        current_pos.y.to_num::<FixedI64>() - next_pos.y.to_num::<FixedI64>(),
    )) {
        turn::toward(world, blocker, wanted, turn::Rate::Walking);
    }

    // This entity's own crossing, without the usual claim juggling.
    move_component.moving_from = current_cell;
    move_component.forgive();
    let new_pos = projection.step_toward(position, next_pos, speed);
    if let Some(wanted) = Facing::of(FixedVec2::new(
        next_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        next_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
    )) {
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    world
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = new_pos;
    if new_pos == next_pos {
        move_component.path.pop();
        if move_component.path.is_empty()
            && cell_walk_done(
                &move_component,
                projection,
                CellPos::from(position),
                shape,
                target,
                size,
                range,
            )
        {
            return OrderState::Finished;
        }
    }
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// The entity holding `cell` against movers of `mask`: a resting or crossing
/// claimant, or a standing footprint covering it. Deterministic — the lowest
/// simulation id wins.
fn claimant_at(world: &mut World, mask: LayerMask, cell: CellPos) -> Option<Entity> {
    for (_, candidate) in world.resource::<EntityIndex>().alive_entries() {
        // Hidden entities hold no cells and their position is stale.
        if world.entity(candidate).contains::<HiddenComponent>() {
            continue;
        }
        let Some(location) = world.entity(candidate).get::<LocationComponent>() else {
            continue;
        };
        let candidate_position = location.position;
        let Some(location_def) = entity_def::of(world, candidate).location else {
            continue;
        };
        if !location_def.solidity().claims_cells()
            || location_def.occupation() & mask == LayerMask::EMPTY
        {
            continue;
        }

        let holds = if is_mid_crossing(candidate_position) {
            // A crossing holds its destination *footprint* — step_claim
            // claimed every entered cell, not the anchor alone.
            world
                .entity(candidate)
                .get::<MoveComponent>()
                .and_then(|movement| movement.path.last().copied())
                .is_some_and(|destination| {
                    CellRect::new(destination, location_def.size()).contains(cell)
                })
        } else {
            CellRect::new(CellPos::from(candidate_position), location_def.size()).contains(cell)
        };
        if holds {
            return Some(candidate);
        }
    }
    None
}

/// Whether some allied mover contesting the same layers is settled within
/// `range` of the goal rect — the continuous walk's peer-arrival evidence
/// that a crowded spot is already taken.
fn resting_ally_within(
    world: &World,
    entity: Entity,
    mask: LayerMask,
    goal: CellRect,
    range: u32,
) -> bool {
    let projection = world.resource::<Map>().projection();
    for (_, candidate) in world.resource::<EntityIndex>().alive_entries() {
        if candidate == entity {
            continue;
        }
        // Hidden entities hold no cells and their position is stale.
        if world.entity(candidate).contains::<HiddenComponent>() {
            continue;
        }
        let Some(location) = world.entity(candidate).get::<LocationComponent>() else {
            continue;
        };
        let def = entity_def::of(world, candidate);
        let Some(location_def) = def.location else {
            continue;
        };
        if !def.can_move()
            || !location_def.solidity().claims_cells()
            || location_def.occupation() & mask == LayerMask::EMPTY
            || world.entity(candidate).contains::<MoveComponent>()
            || !allied(world, entity, candidate)
        {
            continue;
        }
        if projection.in_range_of_rect(CellPos::from(location.position), goal, range) {
            return true;
        }
    }
    false
}

/// Whether both entities have owners the session treats as allies.
fn allied(world: &World, a: Entity, b: Entity) -> bool {
    match (entity_def::owner(world, a), entity_def::owner(world, b)) {
        (Some(first), Some(second)) => world.resource::<GameSession>().are_allied(first, second),
        (None, _) | (_, None) => false,
    }
}

/// The best free cell an idle blocker steps aside to: a whole-footprint step
/// off its own anchor, on its own mask, farthest from the mover's direction
/// of travel, ties broken by cell order. `None` when it is boxed in.
///
/// `toward` is the contested cell the mover is crossing into, which fixes
/// the direction to step away from; `mover` is the mover's standing
/// footprint, which the blocker must not step onto.
fn yield_target(
    world: &World,
    blocker: Entity,
    mover: CellRect,
    toward: CellPos,
) -> Option<CellPos> {
    let location_def = entity_def::of(world, blocker).location?;
    let (mask, size) = (location_def.occupation(), location_def.size());
    let anchor = CellPos::from(entity_def::position(world, blocker));
    let map = world.resource::<Map>();
    let direction_x = toward.x as i64 - mover.origin.x as i64;
    let direction_y = toward.y as i64 - mover.origin.y as i64;

    let mut best: Option<(i64, CellPos)> = None;
    for offset_y in -1i64..=1 {
        for offset_x in -1i64..=1 {
            if offset_x == 0 && offset_y == 0 {
                continue;
            }
            let x = anchor.x as i64 + offset_x;
            let y = anchor.y as i64 + offset_y;
            if x < 0 || y < 0 {
                continue;
            }
            let candidate = CellPos::new(x as u32, y as u32);
            if CellRect::new(candidate, size).intersects(mover)
                || !map.can_step_footprint(mask, size, anchor, candidate)
            {
                continue;
            }
            // The corner rule the blocker's own crossing will enforce.
            if offset_x != 0
                && offset_y != 0
                && !(map.can_step_footprint(
                    mask,
                    size,
                    anchor,
                    CellPos::new(candidate.x, anchor.y),
                ) && map.can_step_footprint(
                    mask,
                    size,
                    anchor,
                    CellPos::new(anchor.x, candidate.y),
                ))
            {
                continue;
            }
            let along = offset_x * direction_x + offset_y * direction_y;
            let better = match best {
                None => true,
                Some((best_along, best_cell)) => {
                    along > best_along || (along == best_along && candidate < best_cell)
                }
            };
            if better {
                best = Some((along, candidate));
            }
        }
    }
    best.map(|(_, cell)| cell)
}

/// Returns `true` if `position` is between two cell origins (a crossing is in progress).
///
/// This infers movement state from the position alone, which is only valid
/// under the lattice invariant documented on
/// [`LocationComponent::position`](crate::components::location::LocationComponent):
/// entities rest exactly on cell origins (integer coordinates), so a
/// fractional component can mean nothing but a crossing in progress.
pub fn is_mid_crossing(position: FixedUVec2) -> bool {
    let cell = CellPos::from(position);
    position != FixedUVec2::from(cell)
}

/// Asserts (debug builds) the lattice invariant for a mid-crossing entity: its
/// position lies on the straight segment between the origin corners of the
/// crossing's departure and target cells, advancing both axes in step on a
/// diagonal.
fn debug_assert_on_crossing(position: FixedUVec2, from: CellPos, to: CellPos) {
    let from = FixedUVec2::from(from);
    let to = FixedUVec2::from(to);
    let on_axis = |value: FixedU64, a: FixedU64, b: FixedU64| {
        if a <= b {
            a <= value && value <= b
        } else {
            b <= value && value <= a
        }
    };
    debug_assert!(
        on_axis(position.x, from.x, to.x) && on_axis(position.y, from.y, to.y),
        "mid-crossing position must lie between the crossing's cells"
    );
    let progress = |value: FixedU64, origin: FixedU64| {
        if value >= origin {
            value - origin
        } else {
            origin - value
        }
    };
    debug_assert!(
        from.x == to.x
            || from.y == to.y
            || progress(position.x, from.x) == progress(position.y, from.y),
        "diagonal crossing must advance both axes in step"
    );
}

/// Inserts MoveComponent to start processing a Move order.
fn insert_driver(entity: Entity, world: &mut World) {
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|loc| loc.position);
    if let Some(position) = position {
        world
            .entity_mut(entity)
            .insert(MoveComponent::new(CellPos::from(position)));
    }
}
