//! Move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{astar, nav_pos::NavPos};

/// Ticks to wait for a blocked cell to clear before recalculating the path.
const BLOCKED_WAIT_TICKS: u32 = 5;

use crate::{
    components::{
        location::LocationComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderState},
        stats::StatsComponent,
    },
    content::stats::StatId,
    entity_def,
    map::Map,
    order::Order,
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

/// Advance a Move order by one tick.
///
/// Each tick:
/// 1. If mid-crossing (position is not at a cell center), continue moving toward
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
pub fn process(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let (target, size, range) = order.move_params().expect("Move order must have params");

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let speed = world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(StatId::SPEED))
        .expect("movable entities have a speed stat");
    let location_def = entity_def::of(world, entity).location.unwrap();
    let occupation = location_def.occupation();

    let Some(mut move_component) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    // Mid-crossing: position is between two cell centers.
    if is_mid_crossing(position) {
        let target_pos = FixedUVec2::from(*move_component.path.last().unwrap());
        let new_pos = step_toward_2d(position, target_pos, speed);
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

    // At rest on a cell — check if the goal is already reached.
    let projection = world.resource::<Map>().projection();
    if astar::in_range_of_rect(
        projection,
        NavPos::from(position),
        NavPos::from(target),
        size,
        range,
    ) {
        return OrderState::Finished;
    }

    // Calculate a path if needed.
    if move_component.path.is_empty() {
        let found = {
            let map = world.resource::<Map>();
            astar::find_path(
                map.nav_grid(),
                projection,
                occupation,
                position,
                target,
                size,
                range,
            )
        };
        match found {
            None => return OrderState::Finished,
            Some(path) if path.is_empty() => return OrderState::Finished,
            Some(path) => {
                // Store reversed so path.last() = next immediate step (pop from back).
                move_component.path = path.into_iter().map(NavPos::from).rev().collect();
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
        if move_component.wait_ticks < BLOCKED_WAIT_TICKS {
            move_component.wait_ticks += 1;
        } else {
            move_component.wait_ticks = 0;
            move_component.path.clear();
        }
        world.entity_mut(entity).insert(move_component);
        return OrderState::InProcessing;
    }
    move_component.wait_ticks = 0;

    // Claim the next cell and release the current one before any position
    // change. Passable entities never claim cells.
    let current_cell = NavPos::from(position);
    if location_def.solidity().claims_cells() {
        let mut map = world.resource_mut::<Map>();
        map.nav_grid_mut()
            .set_occupied_by(occupation, current_cell, false);
        map.nav_grid_mut()
            .set_occupied_by(occupation, next_cell, true);
    }

    move_component.moving_from = current_cell;

    let next_pos = FixedUVec2::from(next_cell);
    let dx = next_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>();
    let dy = next_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>();
    let new_pos = step_toward_2d(position, next_pos, speed);

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

/// Returns `true` if `position` is between two cell centers (a crossing is in progress).
///
/// Cell centers are at integer coordinates. A fractional component means the entity
/// is partway through a crossing.
pub fn is_mid_crossing(position: FixedUVec2) -> bool {
    let cell = NavPos::from(position);
    position != FixedUVec2::from(cell)
}

/// Moves `pos` toward `target` by at most `step` per axis, without overshooting.
fn step_toward_2d(pos: FixedUVec2, target: FixedUVec2, step: FixedU64) -> FixedUVec2 {
    FixedUVec2::new(
        step_toward(pos.x, target.x, step),
        step_toward(pos.y, target.y, step),
    )
}

/// Moves `current` one step toward `target` by at most `step`, without overshooting.
fn step_toward(current: FixedU64, target: FixedU64, step: FixedU64) -> FixedU64 {
    if current < target {
        let next = current + step;
        if next > target { target } else { next }
    } else if current > target {
        if current - target < step {
            target
        } else {
            current - step
        }
    } else {
        current
    }
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
            .insert(MoveComponent::new(NavPos::from(position)));
    }
}
