//! Move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeMap;

use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use ferrets_content::entity_stats::EntityStatId;
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
    shape::Shape,
};
use ferrets_math::{
    FixedI64, FixedU64, facing::Facing, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2,
};
use ferrets_nav_grid::{layer_mask::LayerMask, mover_shape::MoverShape};
use ferrets_pathfinder::{
    astar,
    hierarchy::ClusterPos,
    hpa::{self, Crossing, PathShape, PlanTarget},
    mover_profile::{Blockers, MoverProfile},
};
use ferrets_physics::{
    body::{Bearing, Heading},
    contact,
    ground::Ground,
    motion,
};

use super::{
    orders::{self, Processing, Refusal},
    turn,
};
use crate::{
    components::{
        location::LocationComponent,
        movement::{Landing, Measured, MoveComponent, Pop, Pursued},
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    make_way::{self, Asked, Need, Yielding},
    map::Map,
    movement_model::{self, MovementModel, Settling},
    order::{MoveGoal, Order},
    session::GameSession,
    simulation_id::SimulationId,
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

/// How near an intermediate waypoint counts as reaching it, and how near the
/// body must stand to it after contact for the pop to forgive: half a cell.
const WAYPOINT_POP_SLACK: FixedU64 = FixedU64::lit("0.5");

/// The share of a tick's walk below which a slid step counts as a crawl rather
/// than progress. An exact binary fraction, so scaling a speed by it is exact.
const CRAWL_SHARE: FixedU64 = FixedU64::lit("0.125");

/// The share of a tick's walk a step must keep for its direction to be the
/// body's heading. Larger than [`CRAWL_SHARE`]: a crawl is still real travel and
/// the stall clock has to judge it, while a look wants only the steps the eye
/// can follow.
const HEADING_SHARE: FixedU64 = FixedU64::lit("0.25");

/// How far apart beyond touching a walker still counts as meeting a body, in
/// cells: what rounding leaves between a step stopped at a body's rim and the
/// rim.
const MEETING_SLACK: FixedU64 = FixedU64::lit("0.015625");

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

/// Whether `entity` may start a Move (see [`may_walk`]).
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    may_walk(world, entity)
}

/// Whether `entity` may start an order that walks: its type moves, no
/// breeder holds it in a berth, and it operates.
pub fn may_walk(world: &World, entity: Entity) -> Result<(), Refusal> {
    if !entity_def::of(world, entity).can_move() {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    // One attending a job is let out by the job's order being called off.
    orders::requires_unheld(world, entity)
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
    assert!(
        !entity_def::attached(world, entity),
        "a body attending a job walks only once the job's order lets it out"
    );
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
) -> Processing {
    match policy {
        CancelPolicy::Force => {
            match movement_model::settle_crossing(world, entity) {
                Settling::Settled | Settling::AtRest => {}
            }
            world.entity_mut(entity).remove::<MoveComponent>();
            Processing::state(OrderState::Finished)
        }
        CancelPolicy::Soft => {
            if entry_state == OrderState::InProcessing
                && let Some(mut move_component) =
                    world.entity_mut(entity).get_mut::<MoveComponent>()
            {
                move_component.leave_only_current_target();
                return Processing::state(OrderState::InProcessing);
            }
            // Order has not started yet — nothing to finish, discard immediately.
            Processing::state(OrderState::Finished)
        }
    }
}

/// Whether a Move can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
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
/// no blocked checks — contact is the pushing pass's concern — and its
/// claim follows it onto each footprint its position anchors.
fn process_continuous(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let ordered = order.move_goal().expect("a Move order has a goal");

    let position = entity_def::position(world, entity);
    let speed = entity_def::effective_stat(world, entity, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let shape = entity_def::mover_shape_of(world, entity);
    let (target, goal, range) = goal_of(world, ordered, shape.size);
    let holder = entity_def::simulation_id(world, entity);

    let Some(mut move_component) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    // Crowded off the way: escalations grow the acceptance and bound the
    // grind, like the cell ladder's. Progress means a new record distance
    // to where the walk aims — not to the goal, which a legitimate detour
    // around a lake walks AWAY from for its whole far leg — and motion
    // alone proves nothing, since a body churned around a crowd crosses
    // cells without getting anywhere. Records are strictly decreasing, and
    // the first distance after any change of waypoint only sets one, so a
    // walk that stops closing in escalates within a bounded number of ticks
    // however it is shoved around, while a long way round never does.
    // The mover is judged by its anchor — the cell its claim is stamped on.
    let current_cell = entity_def::location(world, entity).anchor();
    let projection = world.resource::<Map>().projection();
    // Progress is measured toward where the walk aims — the ordered spot on a
    // point order's final leg; with the path empty, a walk within a range
    // toward the middle of its rectangle's first cell.
    let toward =
        entity_def::walk_aim(world, &move_component, Some(ordered), shape.size).unwrap_or(target);
    let distance = position.distance(toward);
    // Last tick's pop is judged by where contact left the body: at the pop it
    // forgives; handed back it forgives nothing.
    match move_component.pursued {
        Pursued::Popped(point) => {
            move_component.land(if position.distance(point) <= WAYPOINT_POP_SLACK {
                Landing::AtPop
            } else {
                Landing::HandedBack
            })
        }
        Pursued::Ordinary | Pursued::Regaining => {}
    }
    let measured = move_component.measure(distance);
    let stalled = match measured {
        Measured::Baseline | Measured::Closer => false,
        Measured::Stalled(ticks) => ticks >= CONTINUOUS_STUCK_TICKS,
    };
    if stalled {
        if move_component.escalate() > GIVE_UP_ESCALATIONS {
            return OrderState::Finished;
        }

        // Peer arrival, as on the cell ladder's rung: a settled ally
        // already inside the acceptance means the spot is taken —
        // arriving beside it kills the pushing contest. Point moves
        // only; a walk within a range finishing short would break its
        // parent order's reach.
        let effective_range = acceptance(ordered, move_component.frustration);
        if ordered.is_position()
            && projection.in_range_of_rect(current_cell, goal, effective_range + 1)
            && resting_ally_within(world, entity, shape.layers, goal, effective_range)
        {
            return OrderState::Finished;
        }

        // Taken goal, as on the cell ladder's rung: the waypoint pursued
        // would end a walk within a range, and a body that is not walking
        // stands on it. Pressing on only pushes against it; a claim-aware
        // route to another cell that serves the goal is taken at once. A
        // stalled point walk is left to its acceptance ring.
        if !ordered.is_position()
            && let Some(&waypoint) = move_component.path.last()
            && cell_arrived(
                world.resource::<Map>(),
                waypoint,
                shape,
                goal,
                ordered,
                move_component.frustration,
            )
            && settled_body_over(world, entity, shape, waypoint)
            && let Some(route) = claim_aware_route(world, shape, holder, current_cell, goal, range)
            && !route.is_empty()
        {
            move_component.take_route(route);
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        }

        // Detour, as on the cell ladder's rung: splice a claim-aware
        // local path around the crowd toward the current waypoint. The
        // claim plane mirrors the standing bodies, and claims cost
        // instead of block, so the mover's own straddled cells cannot
        // wall it in.
        seek_detour(world, &mut move_component, current_cell, shape);
    }
    let effective_range = acceptance(ordered, move_component.frustration);
    // An uncrowded point order lands its middle exactly on the ordered spot, to
    // the bit — an anchored position is just a spot that happens to be on the
    // lattice. Contested spots do not grind forever: the stall clock raises
    // frustration, and a frustrated point move accepts by cells.
    let arrived = match ordered {
        MoveGoal::Position(_) if move_component.frustration == 0 => {
            movement_model::at_point(world, entity, target)
        }
        // A crowded point move accepts its anchor within the frustration
        // ring of the spot.
        MoveGoal::Position(_) => projection.in_range_of_rect(current_cell, goal, effective_range),
        // A walk within a range accepts as every search does, reaching from its
        // footprint, a touch only across an edge it could step.
        MoveGoal::Within { range, .. } => {
            world
                .resource::<Map>()
                .nav_grid()
                .accepts(projection, current_cell, shape, goal, range)
        }
    };
    if arrived {
        return OrderState::Finished;
    }

    let walker = Walker {
        shape,
        holder,
        ordered,
        goal,
        range,
    };

    if move_component.path.is_empty() {
        match onward(world, &mut move_component, &walker, current_cell) {
            Onward::Walking => {}
            Onward::Finished => return OrderState::Finished,
            Onward::Waiting => {
                world.entity_mut(entity).insert(move_component);
                return OrderState::InProcessing;
            }
        }
    }

    // Step toward the aim, but never overlap the body onto a
    // statically blocked cell: a push may have carried the position off the
    // planned cells, and the straight line back would cut corners through
    // footprints. A blocked axis is dropped; a fully blocked step is walled
    // off (below).
    let final_waypoint = move_component.path.len() == 1 && move_component.corridor.is_empty();
    let aim = entity_def::walk_aim(world, &move_component, Some(ordered), shape.size)
        .expect("a walk with a waypoint aims somewhere");
    let desired = projection.step_toward(position, aim, speed);
    let body = entity_def::shape(world, entity);
    // The terrain alone: the walls the step aims into or along.
    let slide = |to: FixedUVec2, world: &World| {
        let grid = world.resource::<Map>().nav_grid();
        motion::advance(to, &Ground::new(grid, shape, body, &[])).point(position)
    };
    let mut new_point = slide(desired, world);
    // Aiming into a wall rather than along it: the slide keeps the step's
    // dominant axis where it can, so losing that one and keeping the minor one
    // means the wall is across the way, not beside it.
    let aimed_into_wall = if desired.x.abs_diff(position.x) >= desired.y.abs_diff(position.y) {
        desired.x != position.x && new_point.x == position.x
    } else {
        desired.y != position.y && new_point.y == position.y
    };
    // Walked along rather than grazed: what is left of a step aimed past a wall
    // is the minor axis's share of it — a fraction of the walk — so the tick is
    // better spent on the way that is open, which is also what clears the block.
    //
    // Decided afresh every tick on purpose: a body held to one axis until it
    // lines up walks itself into the corner, where both axes are shut and the
    // walk stops dead for a tick. Feeling its way around costs a couple of turns
    // and keeps moving.
    let walking_along_wall = aimed_into_wall && {
        // A tick's walk along one axis, stopping at the aim's own
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
        let one_axis = if new_point.x == position.x {
            FixedUVec2::new(position.x, along(position.y, aim.y))
        } else {
            FixedUVec2::new(along(position.x, aim.x), position.y)
        };
        let alongside = slide(one_axis, world);
        // A step newly walled gives way only to a better one, so a wall that
        // costs the body nothing is not walked along at all.
        let further = position.distance(alongside) > position.distance(new_point);
        if further {
            new_point = alongside;
        }
        further
    };
    let traveled = position.distance(new_point);
    // A slide that dropped an axis and kept next to nothing of the step is a
    // body pressed diagonally into a footprint corner: the free axis converges
    // on the corner's tangent line without ever clearing it, and the crumbs it
    // yields keep reading as progress, so the stall clock never fires either.
    // Pressed and crawling is walled, just slower about it.
    let pressed_crawl = new_point != desired && traveled < speed * CRAWL_SHARE;
    if (new_point == position || pressed_crawl) && desired != position {
        // Walled off: a push pressed the body against a footprint, and from
        // this off-lattice spot the straight line to the aim clips the
        // corner. Regain its anchored position — always reachable, since
        // pulling the circle inward only shrinks what it overlaps — then resume
        // the same waypoint, through a claim-aware detour when one exists: the
        // geometry that walled the body off once will wall it off again from
        // the same anchored position, and the pinning crowd shows up in the
        // claim plane, not the static one. The anchored position is held as a
        // real waypoint rather than stepped toward once: a single step can land
        // short, and a replan from short used to walk straight back into the
        // corner — a full-speed ping-pong. Each regain escalates at once, not
        // by the stall clock.
        if move_component.escalate() > GIVE_UP_ESCALATIONS {
            return OrderState::Finished;
        }
        seek_detour(world, &mut move_component, current_cell, shape);
        move_component.regain(current_cell);
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }
    // The bodies near the step, each read as it stands when this walker
    // steps. One that will not give way to a walker is ground to the step,
    // with the terrain, judged in one test (see `motion::advance`): the step
    // slides along its rim, stops where it first touches, or stands, and the
    // stall clock judges the wait.
    let near = entity_def::bodies_near(world, entity, shape.layers, body, new_point);
    let bearing = Bearing::Walking(Heading::toward(FixedVec2::new(
        aim.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        aim.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
    )));
    let obstacles: Vec<Shape> = near
        .iter()
        .filter(|near| !contact::gives_way(near.bearing, bearing, near.meeting))
        .map(|near| near.shape)
        .collect();
    if !obstacles.is_empty() {
        let grid = world.resource::<Map>().nav_grid();
        new_point =
            motion::advance(new_point, &Ground::new(grid, shape, body, &obstacles)).point(position);
    }
    let traveled = position.distance(new_point);
    // The step taken, not the one aimed for: a slide along terrain keeps only
    // what the way allows of the aim, and a body that walks along a bank while
    // looking into it is a body looking where it is not going.
    //
    // Only a step worth seeing is a heading. A crumb — an anchored position
    // being regained, a scrape past a corner — moves the body a pixel or two
    // along some axis it does not mean to travel, and turning down it spins the
    // body over a movement nobody can see. Keeping the previous look is right
    // for a step that landed nowhere too, which has no direction to offer at
    // all.
    if traveled >= speed * HEADING_SHARE
        && let Some(wanted) = Facing::of(FixedVec2::new(
            new_point.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
            new_point.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
        ))
    {
        // Decided on the step the body would really take rather than the one it
        // aimed for, so a walk along a bank is judged by the bank: gating on the
        // aim would stand the body up to face a wall, walk it along, and stand it
        // up again every tick it ran beside one.
        move_component.pivoting = turn::pivoting(world, entity, wanted, move_component.pivoting);
        if move_component.pivoting {
            turn::toward(world, entity, wanted, turn::Rate::Standing);
            // The tick went on coming round, so a tick the stall clock counted
            // is not a tick of getting nowhere — see `MoveComponent::excuse`.
            match measured {
                Measured::Stalled(_) => move_component.excuse(),
                Measured::Baseline | Measured::Closer => {}
            }
            world.entity_mut(entity).insert(move_component);
            return OrderState::InProcessing;
        }
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    // A waypoint a body that is not walking stands on is reached on meeting
    // that body as near the aim as it lets the walker be: the last one of a
    // point order ends the walk there, whoever holds it; an intermediate one
    // is dropped for the next when its holder does not give way to a walker.
    let holds = |near: &entity_def::NearBody| match near.bearing {
        Bearing::Walking(_) => false,
        Bearing::Working | Bearing::Planted | Bearing::Fixed | Bearing::Inert | Bearing::Idle => {
            meets_holder(body, new_point, near.shape, aim)
        }
    };
    // A point order is honored to the bit: its final leg aims at the ordered
    // spot itself, not the goal cell's anchored position.
    let point_order = ordered.is_position();
    if final_waypoint && point_order && near.iter().any(holds) {
        let to = world.resource::<Map>().location(new_point, shape.size);
        movement_model::move_body(world, entity, to);
        return OrderState::Finished;
    }
    let to = world.resource::<Map>().location(new_point, shape.size);
    movement_model::move_body(world, entity, to);
    // Touching the ordered spot completes a point order on the spot, within
    // the same tick: waiting for the next tick's arrival check would let
    // the pushing pass shove the body off first, and a contested point
    // would be touched, lost, and re-walked forever.
    if point_order && new_point == target {
        return OrderState::Finished;
    }
    // A skipped waypoint was the one this tick aimed at, so nothing below has
    // anything left to say about it.
    if !final_waypoint
        && near
            .iter()
            .any(|near| !contact::gives_way(near.bearing, bearing, near.meeting) && holds(near))
    {
        move_component.pop_waypoint(Pop::Skipped(new_point));
        return after_waypoint(world, entity, move_component, &walker);
    }
    // A wall the slide could not be walked along: the way around the corner
    // may still be a clear line somewhere else, which only a search finds.
    // Nothing has gone wrong yet, so this only re-aims.
    if aimed_into_wall && !walking_along_wall && !move_component.detoured && new_point != position {
        seek_detour(world, &mut move_component, current_cell, shape);
        // A splice replaced the pursued waypoint, so the arrival test below
        // has nothing left to say about it — comparing against the stale one
        // would pop the detour's own first cell, the very step that rounds
        // the corner; a fruitless search must not be retried every tick. This
        // tick's step is already committed; the walk resumes next tick.
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }

    // Intermediate waypoints pop within half a cell of slack (see
    // [`WAYPOINT_POP_SLACK`]). The final one pops only on the exact spot, so
    // arrival stays precise — and so does a regained anchored position, whose
    // whole purpose is standing exactly on it.
    let reached = if final_waypoint || matches!(move_component.pursued, Pursued::Regaining) {
        new_point == aim
    } else {
        new_point.distance(aim) <= WAYPOINT_POP_SLACK
    };
    if reached {
        move_component.pop_waypoint(Pop::Reached(aim));
        return after_waypoint(world, entity, move_component, &walker);
    }
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// Advance a Move order by one tick under [`MovementModel::Cell`].
///
/// Each tick:
/// 1. If mid-crossing (between cells), continue moving toward `path.last()`,
///    popping the waypoint on arrival.
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
    let ordered = order.move_goal().expect("a Move order has a goal");

    let standing = entity_def::location(world, entity);
    let position = standing.middle();
    let mid_crossing = standing.is_mid_crossing();
    let current_cell = standing.anchor();
    let speed = entity_def::effective_stat(world, entity, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let shape = entity_def::mover_shape_of(world, entity);
    let (target, goal, range) = goal_of(world, ordered, shape.size);
    let holder = entity_def::simulation_id(world, entity);

    let Some(mut move_component) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    let projection = world.resource::<Map>().projection();

    // Mid-crossing: position is between the crossing's two anchored positions.
    if mid_crossing {
        let crossed_into = *move_component.path.last().unwrap();
        let target_point = anchored_point(world, crossed_into, shape.size);
        debug_assert!(
            movement_model::lies_on_crossing(
                position,
                anchored_point(world, move_component.moving_from, shape.size),
                target_point,
            ),
            "a mid-crossing body lies on its crossing, both axes in step on a diagonal"
        );
        let new_point = projection.step_toward(position, target_point, speed);
        // The look keeps coming round the whole way across, a tick's turn at a
        // time, as it does on every tick of a continuous walk.
        if let Some(wanted) = Facing::of(FixedVec2::new(
            target_point.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
            target_point.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
        )) {
            turn::toward(world, entity, wanted, turn::Rate::Walking);
        }
        step_body(world, entity, new_point);

        if new_point == target_point {
            move_component.path.pop();
            // An emptied path is a spent corridor leg, not necessarily an
            // arrival: a corridor is refined one leg at a time, so the goal
            // decides. Judging it here rather than leaving it to the next
            // tick's at-rest check keeps a walk that really has arrived
            // finishing on the tick it lands.
            if move_component.path.is_empty()
                && cell_walk_done(
                    &move_component,
                    world.resource::<Map>(),
                    crossed_into,
                    shape,
                    goal,
                    ordered,
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
    // into a ring; a walk within a range keeps it exactly — a chase that
    // finished short would break its parent order's reach.
    let arrived = match ordered {
        MoveGoal::Position(_) if move_component.frustration == 0 => {
            movement_model::at_point(world, entity, target)
        }
        MoveGoal::Position(_) | MoveGoal::Within { .. } => cell_arrived(
            world.resource::<Map>(),
            current_cell,
            shape,
            goal,
            ordered,
            move_component.frustration,
        ),
    };
    if arrived {
        return OrderState::Finished;
    }
    // The crowd ladder measures against the same acceptance the arrival check
    // just used.
    let effective_range = acceptance(ordered, move_component.frustration);

    // Continue the plan — or make one — when the current segment runs out.
    if move_component.path.is_empty() {
        let walker = Walker {
            shape,
            holder,
            ordered,
            goal,
            range,
        };
        match next_segment(world, &mut move_component, &walker, current_cell) {
            Segment::Cells(segment) => {
                // Store reversed so path.last() = next immediate step (pop from back).
                move_component.path = segment.into_iter().rev().collect();
            }
            Segment::Ended => return OrderState::Finished,
            Segment::Stale => {
                world.entity_mut(entity).insert(move_component);
                return OrderState::InProcessing;
            }
        }
    }

    // Start a crossing into the next cell.
    let next_cell = *move_component.path.last().unwrap();

    if !world
        .resource::<Map>()
        .can_step_footprint(shape, holder, current_cell, next_cell)
    {
        return resolve_blocked_crossing(
            entity,
            world,
            move_component,
            position,
            current_cell,
            speed,
            shape,
            holder,
            goal,
            ordered,
            range,
            effective_range,
        );
    }
    let next_point = anchored_point(world, next_cell, shape.size);
    let heading = Facing::of(FixedVec2::new(
        next_point.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        next_point.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
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
    // The way ahead is clear only once the crossing starts: a body still
    // turning toward the cell may find it taken again before it sets off.
    move_component.forgive();

    // Claim the footprint entered and release the one left behind, before any
    // position change. Claims live on the grid's unit plane, invisible to the
    // hierarchy.
    movement_model::step_claim(world, entity, next_cell);

    move_component.moving_from = current_cell;

    let new_point = projection.step_toward(position, next_point, speed);

    if let Some(wanted) = heading {
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    step_body(world, entity, new_point);

    if new_point == next_point {
        move_component.path.pop();
        // As above: the spent leg ends the walk only once the goal is met.
        if move_component.path.is_empty()
            && cell_walk_done(
                &move_component,
                world.resource::<Map>(),
                next_cell,
                shape,
                goal,
                ordered,
            )
        {
            return OrderState::Finished;
        }
    }

    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// A walker and what it was ordered: what its plan is asked for.
struct Walker {
    /// The walker's shape.
    shape: MoverShape,
    /// The walker's own claim holder.
    holder: SimulationId,
    /// What it was ordered to walk to.
    ordered: MoveGoal,
    /// The cells the walk closes on.
    goal: CellRect,
    /// How near it closes on them.
    range: u32,
}

/// What a continuous walk with an empty path does next.
enum Onward {
    /// It walks on.
    Walking,
    /// It is over.
    Finished,
    /// It waits a tick for regions refreshed over this tick's changes to plan
    /// from.
    Waiting,
}

/// What a continuous walk does next from `from`, its path empty: a walk wound
/// down is over; a point walk on its goal cell walks to the spot, the rest of
/// its plan dropped; otherwise the next segment of its plan, or a fresh one,
/// is pursued — the walk over when none is left.
fn onward(
    world: &mut World,
    move_component: &mut MoveComponent,
    walker: &Walker,
    from: CellPos,
) -> Onward {
    if move_component.winding_down {
        return Onward::Finished;
    }
    match walker.ordered {
        MoveGoal::Position(_) if from == walker.goal.origin => {
            move_component.pursue_last(walker.goal.origin);
            return Onward::Walking;
        }
        MoveGoal::Position(_) | MoveGoal::Within { .. } => {}
    }
    match next_segment(world, move_component, walker, from) {
        Segment::Cells(segment) => {
            move_component.pursue_segment(segment);
            Onward::Walking
        }
        Segment::Ended => Onward::Finished,
        Segment::Stale => Onward::Waiting,
    }
}

/// The order's state once the waypoint it pursued is reached or dropped: a
/// path emptied with crossings still ahead takes its next segment at once,
/// from where the body stands.
fn after_waypoint(
    world: &mut World,
    entity: Entity,
    mut move_component: MoveComponent,
    walker: &Walker,
) -> OrderState {
    let next = if move_component.path.is_empty() && !move_component.corridor.is_empty() {
        let from = entity_def::location(world, entity).anchor();
        onward(world, &mut move_component, walker, from)
    } else {
        Onward::Walking
    };
    match next {
        Onward::Walking | Onward::Waiting => {
            world.entity_mut(entity).insert(move_component);
            OrderState::InProcessing
        }
        Onward::Finished => OrderState::Finished,
    }
}

/// What planning a walk's next segment gives.
enum Segment {
    /// The cells, walked front to back; never empty.
    Cells(Vec<CellPos>),
    /// No cells: the walker is as close as it can get.
    Ended,
    /// No fresh plan: the walker stands where this tick's changes left the
    /// regions stale.
    Stale,
}

impl Segment {
    /// The segment a search found: its cells, ended when they are none.
    fn of(cells: Vec<CellPos>) -> Self {
        if cells.is_empty() {
            Segment::Ended
        } else {
            Segment::Cells(cells)
        }
    }

    /// The segment a search that may find nothing found.
    fn found(cells: Option<Vec<CellPos>>) -> Self {
        match cells {
            Some(cells) => Segment::of(cells),
            None => Segment::Ended,
        }
    }
}

/// The next segment to walk: the continuation of the current plan, a
/// fresh hierarchical one, or — after a blocked crossing asked for it — a flat
/// claim-aware route. A fresh plan is made only over current regions.
fn next_segment(
    world: &mut World,
    move_component: &mut MoveComponent,
    walker: &Walker,
    from: CellPos,
) -> Segment {
    let Walker {
        shape,
        holder,
        goal,
        range,
        ..
    } = *walker;
    let path_shape = match world.resource::<Map>().movement_model() {
        MovementModel::Cell => PathShape::CellSteps,
        MovementModel::Continuous => PathShape::Waypoints,
    };

    // Continue the standing plan.
    if let Some(plan) = move_component.plan {
        let map = world.resource::<Map>();
        let projection = map.projection();
        if move_component.corridor.is_empty()
            && map
                .nav_grid()
                .accepts(projection, from, shape, plan.rect(), plan.stop)
        {
            // As close as the plan gets; the caller already checked the
            // order's own goal and found it unsatisfied.
            return Segment::Ended;
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
            Some(segment) => return Segment::of(segment),
            None => {
                // The map changed under the plan — replan from scratch.
                move_component.corridor.clear();
                move_component.plan = None;
            }
        }
    }

    // A blocked crossing asked for a claim-aware route around the blocker.
    let projection = world.resource::<Map>().projection();
    if move_component.avoid_claims {
        move_component.avoid_claims = false;
        return Segment::found(claim_aware_route(world, shape, holder, from, goal, range));
    }

    // A fresh plan: reuse the tick's shared corridor for this goal and start
    // locality, planning it once for the whole fanned group. A walker stands
    // in a region of its shape: every set-down is judged, a step enters only
    // anchors its shape fits, a push slides only onto them, no solid footprint
    // goes up over a standing mover, a relocation drops the moved bodies' orders,
    // and regions are refreshed once before the orders run — but an order
    // earlier this tick may have opened the cell a walker has since stepped
    // onto, which the regions learn of only at the next refresh.
    let region = {
        let hierarchy = world.resource::<Map>().hierarchy();
        match hierarchy.region_of(from, shape) {
            Some(region) => region,
            None => {
                assert!(
                    hierarchy.is_dirty_at(from, shape),
                    "a body stands on an anchor in a region of its shape, unless this \
                     tick's changes left its own cluster stale"
                );
                return Segment::Stale;
            }
        }
    };
    let tick = world.resource::<GameSession>().tick();
    let key = ShareKey {
        shape,
        goal: goal.origin,
        goal_size: goal.size,
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
            Some(segment) => return Segment::of(segment),
            None => {
                // A straggler the shared corridor cannot serve plans its own.
                move_component.corridor.clear();
                move_component.plan = None;
            }
        }
    }

    let planned = {
        let map = world.resource::<Map>();
        hpa::plan_corridor(
            map.nav_grid(),
            map.hierarchy(),
            projection,
            from,
            shape,
            PlanTarget {
                cell: goal.origin,
                size: goal.size,
                stop: range,
            },
        )
    };
    let Some((plan_target, corridor)) = planned else {
        return Segment::Ended;
    };
    world
        .resource_mut::<MovePlanShare>()
        .for_tick(tick)
        .insert(key, (plan_target, corridor.clone()));

    move_component.corridor = corridor;
    move_component.plan = Some(plan_target);
    let map = world.resource::<Map>();
    Segment::found(hpa::refine(
        map.nav_grid(),
        map.hierarchy(),
        projection,
        from,
        shape,
        &mut move_component.corridor,
        plan_target,
        path_shape,
    ))
}

/// Where a walk to `goal` aims, the cells it closes on and how near: for a
/// point, the point held so a walker of `walker_size` lies wholly on the map
/// with its middle there, the cell it is anchored at, and `0`; for a
/// rectangle, the middle of its first cell, the rectangle and its range.
fn goal_of(world: &World, goal: MoveGoal, walker_size: CellSize) -> (FixedUVec2, CellRect, u32) {
    match goal {
        MoveGoal::Position(point) => {
            let spot = world.resource::<Map>().location_clamped(point, walker_size);
            (spot.middle(), CellRect::cell(spot.anchor()), 0)
        }
        MoveGoal::Within { rect, range } => (rect.origin.middle(), rect, range),
    }
}

/// A flat route from the anchor `from` to `goal` that treats every unit
/// claim but `holder`'s own as a wall.
fn claim_aware_route(
    world: &World,
    shape: MoverShape,
    holder: SimulationId,
    from: CellPos,
    goal: CellRect,
    range: u32,
) -> Option<Vec<CellPos>> {
    let map = world.resource::<Map>();
    astar::find_path(
        map.nav_grid(),
        map.projection(),
        from,
        MoverProfile::new(shape, Blockers::AllBut(holder.0)),
        goal,
        range,
    )
}

/// The range a walk accepts right now: a point move grows with frustration so
/// a crowd settles into a ring; a walk within a range keeps it exactly.
fn acceptance(ordered: MoveGoal, frustration: u32) -> u32 {
    match ordered {
        MoveGoal::Position(_) => frustration.min(MAX_ACCEPTANCE_GROWTH),
        MoveGoal::Within { range, .. } => range,
    }
}

/// Whether a cell-model walk whose step has just landed ends here: either the
/// goal is satisfied, or the walk is winding down and this was the step it was
/// told to finish.
fn cell_walk_done(
    move_component: &MoveComponent,
    map: &Map,
    cell: CellPos,
    shape: MoverShape,
    goal: CellRect,
    ordered: MoveGoal,
) -> bool {
    move_component.winding_down
        || cell_arrived(map, cell, shape, goal, ordered, move_component.frustration)
}

/// Whether a cell-model walk's goal is satisfied from `cell`: a point move
/// within the frustration ring of its spot, a walk within a range as every
/// search accepts.
fn cell_arrived(
    map: &Map,
    cell: CellPos,
    shape: MoverShape,
    goal: CellRect,
    ordered: MoveGoal,
    frustration: u32,
) -> bool {
    match ordered {
        MoveGoal::Position(_) => {
            map.projection()
                .in_range_of_rect(cell, goal, acceptance(ordered, frustration))
        }
        MoveGoal::Within { range, .. } => {
            map.nav_grid()
                .accepts(map.projection(), cell, shape, goal, range)
        }
    }
}

/// What a settled ally blocking a step does when asked, or what any other
/// blocker does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blocking {
    /// It was asked aside and walks off.
    Leaves,
    /// It stays where it is: it holds, is boxed in, or was not asked.
    Stays,
}

/// Works down the crowd ladder when the next cell of a crossing is held:
/// swap with a head-on counterpart, arrive beside a settled ally, skip a
/// waypoint a settled ally sits on, ask it to step aside, route at once to
/// another goal cell when a body that is not walking and stays holds this
/// one, wait briefly, splice a local detour, and finally throw the plan away
/// for a claim-aware repath. The give-up budget bounds the waits, detours and
/// repaths; the immediate route repeats only when another body settles first
/// on each newly chosen cell.
#[allow(clippy::too_many_arguments)]
fn resolve_blocked_crossing(
    entity: Entity,
    world: &mut World,
    mut move_component: MoveComponent,
    position: FixedUVec2,
    current_cell: CellPos,
    speed: FixedU64,
    shape: MoverShape,
    holder: SimulationId,
    goal: CellRect,
    ordered: MoveGoal,
    range: u32,
    effective_range: u32,
) -> OrderState {
    let next_cell = *move_component.path.last().unwrap();
    let projection = world.resource::<Map>().projection();

    if let Some(blocker) = claimant_at(world, shape, current_cell, next_cell) {
        let blocker_lined_up = !movement_model::is_mid_crossing(world, blocker);

        // Swap: the blocker rests on my next cell and wants mine — the
        // head-on case. Both crossings run at once; no claim bit changes
        // hands visibly, each side keeps exactly its own claim — which is
        // only an identity when both occupy the same layers **and the same
        // footprint**, so unequal shapes fall through to the other rungs:
        // exchanging a 1×1 claim for a 2×2 one would leave ghost-claimed
        // cells behind one body and unclaimed cells under the other. Each
        // side lands on a cell of its own route, ground its own domain takes,
        // so bodies of different domains trade places as equals do.
        if blocker_lined_up
            && {
                let form = entity_def::form_of(world, blocker);
                form.occupies() == shape.layers && form.extent() == shape.size
            }
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
                goal,
                ordered,
            );
        }

        // A settled ally: at rest, idle and able to move, whether or not it
        // makes way when asked.
        let owner = entity_def::owner(world, entity);
        let settled_ally = blocker_lined_up
            && entity_def::idle(world, blocker)
            && entity_def::of(world, blocker).can_move()
            && world
                .resource::<GameSession>()
                .relation(owner, entity_def::owner(world, blocker))
                .is_friendly();

        let blocking = if settled_ally {
            // Peer arrival: a settled ally already inside my acceptance
            // means the spot is taken — arriving beside it kills the
            // pushing contest. Point moves only; a walk within a range
            // finishing short would break its parent order's reach.
            if ordered.is_position()
                && projection.in_range_of_rect(next_cell, goal, effective_range)
                && projection.in_range_of_rect(current_cell, goal, effective_range + 1)
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

            // Yield: one that makes way is asked off the cells the walk still
            // enters along its segment, so it does not step aside onto the way
            // ahead, and the walk waits below while it walks off; one boxed
            // in, or holding, stays.
            match make_way::makes_way(world, blocker, owner) {
                Yielding::Yields => {
                    let mut ahead = entered_cells(shape, current_cell, next_cell);
                    for &cell in move_component.path.iter().rev().skip(1) {
                        ahead.extend(
                            CellRect::new(cell, shape.size)
                                .cells()
                                .filter(|cell| !ahead.contains(cell))
                                .collect::<Vec<_>>(),
                        );
                    }
                    let need = Need::Passage {
                        ahead: &ahead,
                        heading: (current_cell, next_cell),
                    };
                    match make_way::ask(world, blocker, owner, need) {
                        Asked::Leaves => Blocking::Leaves,
                        Asked::Boxed => Blocking::Stays,
                    }
                }
                Yielding::Holds => Blocking::Stays,
            }
        } else {
            Blocking::Stays
        };
        let taken_goal = match blocking {
            Blocking::Leaves => false,
            Blocking::Stays => {
                blocker_lined_up
                    && !entity_def::walking(world, blocker)
                    && cell_arrived(
                        world.resource::<Map>(),
                        next_cell,
                        shape,
                        goal,
                        ordered,
                        move_component.frustration,
                    )
            }
        };
        if taken_goal {
            // Taken goal: the step would end the walk, and the cell is held by
            // a body that is not walking and stays — one at work, one that
            // cannot move, one not allied, or an ally that holds or is boxed
            // in. A point walk ends beside it, as on a settled ally. A walk
            // within a range takes a claim-aware route to another cell that
            // serves the goal at once, since detouring back onto that same
            // cell only stalls; with none, it waits as on any held step.
            if ordered.is_position() {
                return OrderState::Finished;
            }
            if let Some(route) = claim_aware_route(world, shape, holder, current_cell, goal, range)
                && !route.is_empty()
            {
                move_component.take_route(route);
                world.entity_mut(entity).insert(move_component);
                return OrderState::InProcessing;
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
/// the blocker toward `current_cell`, each taking the other's claim.
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
    goal: CellRect,
    ordered: MoveGoal,
) -> OrderState {
    movement_model::swap_claims(world, entity, blocker);
    let next_point = anchored_point(world, next_cell, shape.size);
    let current_point = anchored_point(world, current_cell, shape.size);

    // The counterpart's crossing starts here; its own order processing
    // continues it as any other mid-crossing tick.
    let blocker_speed = entity_def::effective_stat(world, blocker, EntityStatId::SPEED)
        .expect("movable entities have a speed stat");
    let blocker_new = projection.step_toward(next_point, current_point, blocker_speed);
    {
        let mut blocker_mut = world.entity_mut(blocker);
        let mut counter_move = blocker_mut.get_mut::<MoveComponent>().unwrap();
        counter_move.moving_from = next_cell;
        counter_move.forgive();
        if blocker_new == current_point {
            counter_move.path.pop();
        }
    }
    step_body(world, blocker, blocker_new);
    // Both halves of a swap are crossings, so both come round as they walk.
    // Neither holds still first: the two have already agreed to trade places,
    // and a body standing to turn would leave the other walking into it.
    if let Some(wanted) = Facing::of(FixedVec2::new(
        current_point.x.to_num::<FixedI64>() - next_point.x.to_num::<FixedI64>(),
        current_point.y.to_num::<FixedI64>() - next_point.y.to_num::<FixedI64>(),
    )) {
        turn::toward(world, blocker, wanted, turn::Rate::Walking);
    }

    // This entity's own crossing; its claim was traded above.
    move_component.moving_from = current_cell;
    move_component.forgive();
    let new_point = projection.step_toward(position, next_point, speed);
    if let Some(wanted) = Facing::of(FixedVec2::new(
        next_point.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        next_point.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
    )) {
        turn::toward(world, entity, wanted, turn::Rate::Walking);
    }
    step_body(world, entity, new_point);
    if new_point == next_point {
        move_component.path.pop();
        if move_component.path.is_empty()
            && cell_walk_done(
                &move_component,
                world.resource::<Map>(),
                next_cell,
                shape,
                goal,
                ordered,
            )
        {
            return OrderState::Finished;
        }
    }
    world.entity_mut(entity).insert(move_component);
    OrderState::InProcessing
}

/// Seeks the blockage's one local detour from `from` toward the waypoint
/// `walk` pursues, a claim-aware path for `shape`: spliced in when one is
/// found, and the detour spent either way — with no waypoint pursued, there
/// is nothing to seek and it is spent all the same. Nothing is sought when
/// the detour is already spent.
fn seek_detour(world: &World, walk: &mut MoveComponent, from: CellPos, shape: MoverShape) {
    if walk.detoured {
        return;
    }
    let Some(&waypoint) = walk.path.last() else {
        walk.detoured = true;
        return;
    };
    let map = world.resource::<Map>();
    let cells = hpa::detour(
        map.nav_grid(),
        map.hierarchy(),
        map.projection(),
        from,
        shape,
        waypoint,
    );
    match cells.filter(|cells| !cells.is_empty()) {
        Some(cells) => walk.splice_detour(cells),
        None => walk.detoured = true,
    }
}

/// Whether a walker whose circle is `body`, standing at `at`, meets the
/// `holder` circle and the holder stands on `spot`: the walker standing on
/// the spot would overlap it, and where the walker stands the two circles
/// touch or overlap and the walker is no farther from the spot than touching
/// a holder standing on it, each within [`MEETING_SLACK`].
fn meets_holder(body: Shape, at: FixedUVec2, holder: Shape, spot: FixedUVec2) -> bool {
    let radius = |shape: Shape| shape.radius().expect("only circles hold a walker's spot");
    let touching = radius(body) + radius(holder);
    let middle = holder.middle();
    spot.distance(middle) < touching
        && at.distance(middle) <= touching + MEETING_SLACK
        && at.distance(spot) <= touching + MEETING_SLACK
}

/// The entity holding a cell that a step of `shape` from `from` to `to`
/// enters: one whose mark on the grid on the step's layers — a claim, or a
/// static body's stamp ([`entity_def::claim_on`]) — covers it.
/// Deterministic — the lowest simulation id wins.
fn claimant_at(world: &World, shape: MoverShape, from: CellPos, to: CellPos) -> Option<Entity> {
    let entered = entered_cells(shape, from, to);
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, candidate)| candidate)
        .find(|&candidate| {
            entity_def::claim_on(world, candidate, shape.layers)
                .is_some_and(|claim| entered.iter().any(|&cell| claim.contains(cell)))
        })
}

/// Whether a body other than `entity`, claiming cells on the layers of
/// `shape` and not walking, stands where a footprint of `shape` anchored at
/// `cell` would — read at the cell each body's claim is stamped on, as the
/// continuous model's claim plane is.
fn settled_body_over(world: &World, entity: Entity, shape: MoverShape, cell: CellPos) -> bool {
    let footprint = CellRect::new(cell, shape.size);
    world
        .resource::<EntityIndex>()
        .alive_iter()
        .any(|(_, &candidate)| {
            candidate != entity
                && !entity_def::walking(world, candidate)
                && entity_def::claim_on(world, candidate, shape.layers)
                    .is_some_and(|claim| claim.intersects(footprint))
        })
}

/// Whether some idle allied mover contesting the same layers is settled
/// within `range` of the goal rect — the continuous walk's peer-arrival
/// evidence that a crowded spot is already taken.
fn resting_ally_within(
    world: &World,
    entity: Entity,
    layers: LayerMask,
    goal: CellRect,
    range: u32,
) -> bool {
    let projection = world.resource::<Map>().projection();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, candidate)| candidate)
        .any(|candidate| {
            candidate != entity
                && entity_def::of(world, candidate).can_move()
                && !entity_def::walking(world, candidate)
                && entity_def::idle(world, candidate)
                && entity_def::relation(world, entity, candidate).is_friendly()
                && entity_def::claim_on(world, candidate, layers)
                    .is_some_and(|claim| projection.in_range_of_rect(claim.origin, goal, range))
        })
}

/// The cells a step of `shape` from `from` to `to` enters: those of the
/// footprint at `to` outside the one at `from`.
fn entered_cells(shape: MoverShape, from: CellPos, to: CellPos) -> Vec<CellPos> {
    CellRect::new(to, shape.size)
        .minus(CellRect::new(from, shape.size))
        .flat_map(CellRect::cells)
        .collect()
}

/// Inserts MoveComponent to start processing a Move order.
fn insert_driver(entity: Entity, world: &mut World) {
    if world.entity(entity).contains::<LocationComponent>() {
        let anchor = entity_def::location(world, entity).anchor();
        world.entity_mut(entity).insert(MoveComponent::new(anchor));
    }
}

/// The middle of a body of `size` anchored at `cell`.
fn anchored_point(world: &World, cell: CellPos, size: CellSize) -> FixedUVec2 {
    world.resource::<Map>().location_at(cell, size).middle()
}

/// Moves the cell-model `entity`'s body along its crossing so its middle is at
/// `point`.
fn step_body(world: &mut World, entity: Entity, point: FixedUVec2) {
    let location = world
        .resource::<Map>()
        .location(point, entity_def::extent(world, entity));
    entity_def::set_location(world, entity, location);
}
