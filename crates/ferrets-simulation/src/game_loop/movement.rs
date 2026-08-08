//! Move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use std::collections::BTreeMap;

use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{
    astar,
    hierarchy::ClusterPos,
    hpa::{self, Crossing, PathShape, PlanTarget},
    layer_mask::LayerMask,
};
use ferrets_physics::{body, terrain};

/// Ticks to wait for a blocked cell to clear before recalculating the path.
const BLOCKED_WAIT_TICKS: u32 = 5;

/// How many cells ahead a local detour rejoins its segment.
const DETOUR_LOOKAHEAD: usize = 6;

/// How far the acceptance range grows as a mover keeps failing to close in,
/// so a crowded destination settles into a ring instead of a grinding queue.
const MAX_ACCEPTANCE_GROWTH: u32 = 4;

/// Blockage escalations a walk may burn before giving up where it stands.
const GIVE_UP_ESCALATIONS: u32 = 8;

/// One shared plan's identity: the mover mask's bits, the goal (cell,
/// footprint, stop distance), and the start's cluster and region.
type ShareKey = (u32, CellPos, (u32, u32), u32, ClusterPos, u32);

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

use crate::{
    components::{
        entity_stats::StatsComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        owner::OwnerComponent,
    },
    content::entity_stats::EntityStatId,
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    session::GameSession,
};

/// Called once when a Move order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing` if the move should proceed,
/// or `Finished` immediately if the entity cannot move. Whether the target is already
/// reached is not checked here — that is deferred to [`process`].
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if !entity_def::of(world, entity).can_move() {
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
pub fn process(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => process_cell(entity, order, world),
        MovementModel::Continuous => process_continuous(entity, order, world),
    }
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

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let speed = world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(EntityStatId::SPEED))
        .expect("movable entities have a speed stat");
    let location_def = entity_def::of(world, entity).location.unwrap();
    let occupation = location_def.occupation();

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
    // The mover is judged by the cell under its body's center — the cell
    // it visually stands on and claims. Its anchor's floor sits one cell
    // short whenever a push pressed it against what it walks toward.
    let current_cell = body::center_cell(position);
    let projection = world.resource::<Map>().projection();
    let pursued = move_component
        .path
        .last()
        .map_or(target, |&waypoint| FixedUVec2::from(waypoint));
    let distance = position.distance(pursued);
    if distance < move_component.best_distance {
        move_component.best_distance = distance;
        move_component.wait_ticks = 0;
        move_component.frustration = 0;
        move_component.detoured = false;
    } else {
        move_component.wait_ticks += 1;
        if move_component.wait_ticks >= CONTINUOUS_STUCK_TICKS {
            move_component.wait_ticks = 0;
            move_component.frustration += 1;
            if move_component.frustration > GIVE_UP_ESCALATIONS {
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
                && resting_ally_within(world, entity, occupation, goal, effective_range)
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
                        occupation,
                        current_cell,
                        waypoint,
                    )
                };
                if let Some(cells) = cells
                    && !cells.is_empty()
                {
                    move_component.detoured = true;
                    move_component.path.pop();
                    move_component.path.extend(cells.into_iter().rev());
                    move_component.best_distance = FixedU64::MAX;
                }
            }
        }
    }
    let effective_range = acceptance(range, move_component.frustration);
    // An uncrowded point order lands exactly on the ordered spot, to the
    // bit — a lattice target is just a spot that happens to be a cell
    // origin. Contested spots do not grind forever: the stall clock raises
    // frustration, and a frustrated walk accepts by cells like everything
    // ranged or footprinted.
    let arrived = if range == 0 && size == CellSize::ONE && move_component.frustration == 0 {
        position == target
    } else {
        projection.in_range_of_rect(
            current_cell,
            CellRect::new(CellPos::from(target), size),
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
        if point_order && current_cell == CellPos::from(target) {
            // Already standing on the goal cell — a plan would be empty;
            // walk the last sub-cell stretch to the spot directly.
            move_component.path.push(CellPos::from(target));
            move_component.best_distance = FixedU64::MAX;
        } else {
            match next_segment(
                world,
                &mut move_component,
                occupation,
                position,
                target,
                size,
                range,
            ) {
                None => return OrderState::Finished,
                Some(segment) if segment.is_empty() => return OrderState::Finished,
                Some(segment) => {
                    move_component.path = segment.into_iter().rev().collect();
                    // A fresh segment pursues a fresh waypoint.
                    move_component.best_distance = FixedU64::MAX;
                }
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
    let new_pos = terrain::slide_toward(
        world.resource::<Map>().nav_grid(),
        occupation,
        position,
        desired,
        radius,
    );
    if new_pos == position && desired != position {
        // Fully walled off: a push pressed the body against a footprint,
        // and from this off-lattice spot even a fresh plan's first line
        // clips the corner. Regain the cell's own lattice point — always
        // reachable, since pulling the circle inward only shrinks what it
        // overlaps — and replan from there, where cell-resolution plans
        // are body-safe.
        let regained = projection.step_toward(position, FixedUVec2::from(current_cell), speed);
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .unwrap()
            .position = regained;
        move_component.path.clear();
        move_component.best_distance = FixedU64::MAX;
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }
    {
        let mut entity_mut = world.entity_mut(entity);
        let mut location = entity_mut.get_mut::<LocationComponent>().unwrap();
        let facing = FixedVec2::new(
            waypoint.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
            waypoint.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
        );
        // Standing exactly on the waypoint gives no direction; keep the
        // previous facing rather than zeroing it.
        if facing != FixedVec2::ZERO {
            location.facing = facing;
        }
        location.position = new_pos;
    }
    // Touching the ordered spot completes a point order on the spot, within
    // the same tick: waiting for the next tick's arrival check would let
    // the pushing pass shove the body off first, and a contested point
    // would be touched, lost, and re-walked forever.
    if point_order && new_pos == target {
        return OrderState::Finished;
    }
    // Intermediate waypoints pop within half a body of slack — pushes make
    // exact hits rare, and the next waypoint pulls from far ahead anyway.
    // The final one pops only on the exact spot, so arrival stays precise.
    let reached = if final_waypoint {
        new_pos == waypoint
    } else {
        new_pos.distance(waypoint) <= FixedU64::from_num(0.5)
    };
    if reached {
        // A consumed waypoint is progress; the next one starts its own
        // record.
        move_component.path.pop();
        move_component.best_distance = FixedU64::MAX;
        move_component.wait_ticks = 0;
        move_component.frustration = 0;
        move_component.detoured = false;
    }
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// Advance a Move order by one tick under [`MovementModel::Cell`].
///
/// Each tick:
/// 1. If mid-crossing (position is not on a cell origin), continue moving toward
///    `path.last()`. Pop the waypoint on arrival; return `Finished` if path is empty.
/// 2. Otherwise finish if the goal is within the order's stop distance, or
///    calculate a path if needed, claim the next cell in the nav grid, and begin
///    the crossing.
///
/// The nav grid occupation for a cell is claimed when the entity **starts** crossing
/// into it, not on arrival. `path.last()` is the active crossing target; it is popped
/// only when the entity arrives, so no separate current-target field is needed.
///
/// `MoveComponent` is taken from the entity at the start and reinserted on
/// `InProcessing`; on `Finished` it is simply dropped (removed by not reinserting).
fn process_cell(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let (target, size, range) = order.move_params().expect("Move order must have params");

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let speed = world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(EntityStatId::SPEED))
        .expect("movable entities have a speed stat");
    let location_def = entity_def::of(world, entity).location.unwrap();
    let occupation = location_def.occupation();

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
            if move_component.path.is_empty() {
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
    let effective_range = acceptance(range, move_component.frustration);
    if projection.in_range_of_rect(
        CellPos::from(position),
        CellRect::new(CellPos::from(target), size),
        effective_range,
    ) {
        return OrderState::Finished;
    }

    // Continue the plan — or make one — when the current segment runs out.
    if move_component.path.is_empty() {
        match next_segment(
            world,
            &mut move_component,
            occupation,
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

    if world
        .resource::<Map>()
        .nav_grid()
        .is_occupied_by(occupation, next_cell)
    {
        return resolve_blocked_crossing(
            entity,
            world,
            move_component,
            position,
            speed,
            occupation,
            target,
            size,
            range,
            effective_range,
        );
    }
    move_component.wait_ticks = 0;
    move_component.frustration = 0;
    move_component.detoured = false;

    // Claim the next cell and release the current one before any position
    // change. Claims live on the grid's unit plane, invisible to the
    // hierarchy. Passable entities never claim cells.
    let current_cell = CellPos::from(position);
    if location_def.solidity().claims_cells() {
        let mut map = world.resource_mut::<Map>();
        // One claimant per cell per layer is the cell model's contract:
        // a release must find the cell held, a claim must find it free —
        // a violation means somebody's claim was silently overwritten.
        debug_assert!(
            map.nav_grid().is_claimed_by(occupation, current_cell),
            "a crossing must release the cell the mover holds"
        );
        debug_assert!(
            !map.nav_grid().is_claimed_by(occupation, next_cell),
            "a crossing must claim a free cell"
        );
        map.nav_grid_mut()
            .set_claimed_by(occupation, current_cell, false);
        map.nav_grid_mut()
            .set_claimed_by(occupation, next_cell, true);
    }

    move_component.moving_from = current_cell;

    let next_pos = FixedUVec2::from(next_cell);
    let dx = next_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>();
    let dy = next_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>();
    let new_pos = projection.step_toward(position, next_pos, speed);

    {
        let mut entity_mut = world.entity_mut(entity);
        let mut location_component = entity_mut.get_mut::<LocationComponent>().unwrap();
        location_component.facing = FixedVec2::new(dx, dy);
        location_component.position = new_pos;
    }

    if new_pos == next_pos {
        move_component.path.pop();
        if move_component.path.is_empty() {
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
    occupation: LayerMask,
    position: FixedUVec2,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
) -> Option<Vec<CellPos>> {
    let from = body::center_cell(position);
    let shape = match world.resource::<Map>().movement_model() {
        MovementModel::Cell => PathShape::CellSteps,
        MovementModel::Continuous => PathShape::Waypoints,
    };

    // Continue the standing plan.
    if let Some(plan) = move_component.plan {
        let map = world.resource::<Map>();
        let projection = map.projection();
        if move_component.corridor.is_empty()
            && projection.in_range_of_rect(from, CellRect::new(plan.cell, plan.size), plan.stop)
        {
            // As close as the plan gets; the caller already checked the
            // order's own goal and found it unsatisfied.
            return None;
        }
        let refined = hpa::refine(
            map.nav_grid(),
            map.hierarchy(),
            projection,
            occupation,
            from,
            &mut move_component.corridor,
            plan,
            shape,
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
        let region = if map.hierarchy().serves(occupation) {
            map.hierarchy().region_of(occupation, from)
        } else {
            None
        };
        (map.projection(), map.hierarchy().serves(occupation), region)
    };
    if move_component.avoid_claims || !serves {
        move_component.avoid_claims = false;
        let map = world.resource::<Map>();
        return astar::find_path(
            map.nav_grid(),
            projection,
            occupation,
            position,
            target,
            size,
            range,
        )
        .map(|path| path.into_iter().map(CellPos::from).collect());
    }

    // A fresh plan: reuse the tick's shared corridor for this goal and start
    // locality, planning it once for the whole fanned group.
    let region = region?;
    let tick = world.resource::<GameSession>().tick();
    let key: ShareKey = {
        let map = world.resource::<Map>();
        (
            *occupation,
            CellPos::from(target),
            (size.width, size.height),
            range,
            map.hierarchy().cluster_of(from),
            region,
        )
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
            occupation,
            from,
            &mut move_component.corridor,
            plan_target,
            shape,
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
            occupation,
            from,
            CellPos::from(target),
            size,
            range,
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
        occupation,
        from,
        &mut move_component.corridor,
        plan_target,
        shape,
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
    occupation: LayerMask,
    target: FixedUVec2,
    size: CellSize,
    range: u32,
    effective_range: u32,
) -> OrderState {
    let current_cell = CellPos::from(position);
    let next_cell = *move_component.path.last().unwrap();
    let projection = world.resource::<Map>().projection();

    if let Some(blocker) = claimant_at(world, occupation, next_cell) {
        let blocker_position = world
            .entity(blocker)
            .get::<LocationComponent>()
            .unwrap()
            .position;
        let blocker_at_rest = !is_mid_crossing(blocker_position);

        // Swap: the blocker rests on my next cell and wants mine — the
        // head-on case. Both crossings run at once; no claim bit changes
        // hands visibly, each side keeps exactly one cell claimed — which
        // is only an identity when both occupy the same layers, so unequal
        // masks fall through to the other rungs instead of leaking the
        // wider mask's bits on the exchanged cells.
        if blocker_at_rest
            && entity_def::of(world, blocker)
                .location
                .is_some_and(|location| location.occupation() == occupation)
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
                current_cell,
                next_cell,
                blocker,
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
            if let Some(aside) = yield_target(world, occupation, next_cell, current_cell) {
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
    move_component.wait_ticks = 0;
    move_component.frustration += 1;
    if move_component.frustration > GIVE_UP_ESCALATIONS {
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
                occupation,
                current_cell,
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
    move_component.path.clear();
    move_component.corridor.clear();
    move_component.plan = None;
    move_component.avoid_claims = true;
    move_component.detoured = false;
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
    current_cell: CellPos,
    next_cell: CellPos,
    blocker: Entity,
) -> OrderState {
    let next_pos = FixedUVec2::from(next_cell);
    let current_pos = FixedUVec2::from(current_cell);

    // The counterpart's crossing starts here; its own order processing
    // continues it as any other mid-crossing tick.
    let blocker_speed = world
        .entity(blocker)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(EntityStatId::SPEED))
        .expect("movable entities have a speed stat");
    let blocker_new = projection.step_toward(next_pos, current_pos, blocker_speed);
    {
        let mut blocker_mut = world.entity_mut(blocker);
        let mut counter_move = blocker_mut.get_mut::<MoveComponent>().unwrap();
        counter_move.moving_from = next_cell;
        counter_move.wait_ticks = 0;
        counter_move.frustration = 0;
        counter_move.detoured = false;
        if blocker_new == current_pos {
            counter_move.path.pop();
        }
        let mut blocker_location = blocker_mut.get_mut::<LocationComponent>().unwrap();
        blocker_location.facing = FixedVec2::new(
            current_pos.x.to_num::<FixedI64>() - next_pos.x.to_num::<FixedI64>(),
            current_pos.y.to_num::<FixedI64>() - next_pos.y.to_num::<FixedI64>(),
        );
        blocker_location.position = blocker_new;
    }

    // This entity's own crossing, without the usual claim juggling.
    move_component.moving_from = current_cell;
    move_component.wait_ticks = 0;
    move_component.frustration = 0;
    move_component.detoured = false;
    let new_pos = projection.step_toward(position, next_pos, speed);
    {
        let mut entity_mut = world.entity_mut(entity);
        let mut location = entity_mut.get_mut::<LocationComponent>().unwrap();
        location.facing = FixedVec2::new(
            next_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
            next_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
        );
        location.position = new_pos;
    }
    if new_pos == next_pos {
        move_component.path.pop();
        if move_component.path.is_empty() {
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
            world
                .entity(candidate)
                .get::<MoveComponent>()
                .and_then(|movement| movement.path.last().copied())
                == Some(cell)
        } else {
            let origin = CellPos::from(candidate_position);
            let CellSize { width, height } = location_def.size();
            cell.x >= origin.x
                && cell.x < origin.x + width
                && cell.y >= origin.y
                && cell.y < origin.y + height
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
    let owner = |entity: Entity| {
        world
            .entity(entity)
            .get::<OwnerComponent>()
            .map(|component| component.player())
    };
    match (owner(a), owner(b)) {
        (Some(first), Some(second)) => world.resource::<GameSession>().are_allied(first, second),
        (None, _) | (_, None) => false,
    }
}

/// The best free cell an idle blocker at `blocker_cell` steps aside to:
/// farthest from the mover's direction of travel, ties broken by cell order.
/// `None` when it is boxed in.
fn yield_target(
    world: &World,
    mask: LayerMask,
    blocker_cell: CellPos,
    mover_cell: CellPos,
) -> Option<CellPos> {
    let map = world.resource::<Map>();
    let grid = map.nav_grid();
    let direction_x = blocker_cell.x as i64 - mover_cell.x as i64;
    let direction_y = blocker_cell.y as i64 - mover_cell.y as i64;

    let mut best: Option<(i64, CellPos)> = None;
    for offset_y in -1i64..=1 {
        for offset_x in -1i64..=1 {
            if offset_x == 0 && offset_y == 0 {
                continue;
            }
            let x = blocker_cell.x as i64 + offset_x;
            let y = blocker_cell.y as i64 + offset_y;
            if x < 0 || y < 0 {
                continue;
            }
            let candidate = CellPos::new(x as u32, y as u32);
            if candidate == mover_cell || !grid.is_passable_by(mask, candidate) {
                continue;
            }
            // The corner rule the blocker's own crossing will enforce.
            if offset_x != 0
                && offset_y != 0
                && !(grid.is_passable_by(mask, CellPos::new(candidate.x, blocker_cell.y))
                    && grid.is_passable_by(mask, CellPos::new(blocker_cell.x, candidate.y)))
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
