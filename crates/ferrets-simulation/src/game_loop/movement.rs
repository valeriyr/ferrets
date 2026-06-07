//! Move order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{
    astar::{self, Projection},
    nav_pos::NavPos,
};

/// Ticks to wait for a blocked cell to clear before recalculating the path.
const BLOCKED_WAIT_TICKS: u32 = 5;

use crate::{
    components::{
        location::{LocationComponent, LocationStaticData},
        movement::{MoveComponent, MoveStaticData},
        order_queue::{CancelPolicy, OrderState},
    },
    map::Map,
    order::Order,
};

/// Called once when a Move order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing` if the move should proceed,
/// or `Finished` if it can be skipped immediately (e.g. already at target — not yet
/// checked here, deferred to [`process`]).
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    insert_driver(entity, world);
    OrderState::InProcessing
}

/// Called when a Move order resumes from `Suspended` (its sub-order just finished).
///
/// Inserts the driver component and returns `InProcessing`.
pub fn prepare_suspended(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
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
                && let Some(mut mc) = world.entity_mut(entity).get_mut::<MoveComponent>()
            {
                mc.leave_only_current_target();
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
/// 2. Otherwise calculate a path if needed, claim the next cell in the nav grid,
///    and begin the crossing.
///
/// The nav grid occupation for a cell is claimed when the entity **starts** crossing
/// into it, not on arrival. `path.last()` is the active crossing target; it is popped
/// only when the entity arrives, so no separate current-target field is needed.
///
/// `MoveComponent` is taken from the entity at the start and reinserted on
/// `InProcessing`; on `Finished` it is simply dropped (removed by not reinserting).
pub fn process(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target = order.move_target().expect("Move order must have a target");

    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let speed = world
        .entity(entity)
        .get::<MoveStaticData>()
        .unwrap()
        .speed();
    let occupation = world
        .entity(entity)
        .get::<LocationStaticData>()
        .unwrap()
        .occupation();

    let Some(mut mc) = world.entity_mut(entity).take::<MoveComponent>() else {
        return OrderState::Finished;
    };

    // Mid-crossing: position is between two cell centers.
    if is_mid_crossing(position) {
        let target_pos = FixedUVec2::from(*mc.path.last().unwrap());
        let new_pos = step_toward_2d(position, target_pos, speed);
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .unwrap()
            .position = new_pos;

        if new_pos == target_pos {
            mc.path.pop();
            if mc.path.is_empty() {
                return OrderState::Finished;
            }
        }
        world.entity_mut(entity).insert(mc);
        return OrderState::InProcessing;
    }

    // At rest on a cell — check if the goal is already reached.
    if NavPos::from(position) == NavPos::from(target) {
        return OrderState::Finished;
    }

    // Calculate a path if needed.
    if mc.path.is_empty() {
        let found = {
            let map = world.resource::<Map>();
            astar::find_path(
                map.nav_grid(),
                Projection::Isometric,
                occupation,
                position,
                target,
                0,
            )
        };
        match found {
            None => return OrderState::Finished,
            Some(p) if p.is_empty() => return OrderState::Finished,
            Some(p) => {
                // Store reversed so path.last() = next immediate step (pop from back).
                mc.path = p.into_iter().map(NavPos::from).rev().collect();
            }
        }
    }

    // Start a crossing into the next cell.
    let next_cell = *mc.path.last().unwrap();

    if world
        .resource::<Map>()
        .nav_grid()
        .is_occupied_by(occupation, next_cell)
    {
        if mc.wait_ticks < BLOCKED_WAIT_TICKS {
            mc.wait_ticks += 1;
        } else {
            mc.wait_ticks = 0;
            mc.path.clear();
        }
        world.entity_mut(entity).insert(mc);
        return OrderState::InProcessing;
    }
    mc.wait_ticks = 0;

    // Claim the next cell and release the current one before any position change.
    let current_cell = NavPos::from(position);
    {
        let mut map = world.resource_mut::<Map>();
        map.nav_grid_mut()
            .set_occupied_by(occupation, current_cell, false);
        map.nav_grid_mut()
            .set_occupied_by(occupation, next_cell, true);
    }

    mc.moving_from = current_cell;

    let next_pos = FixedUVec2::from(next_cell);
    let dx = next_pos.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>();
    let dy = next_pos.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>();
    let new_pos = step_toward_2d(position, next_pos, speed);

    {
        let mut entity_mut = world.entity_mut(entity);
        let mut loc = entity_mut.get_mut::<LocationComponent>().unwrap();
        loc.facing = FixedVec2::new(dx, dy);
        loc.position = new_pos;
    }

    if new_pos == next_pos {
        mc.path.pop();
        if mc.path.is_empty() {
            return OrderState::Finished;
        }
    }

    world.entity_mut(entity).insert(mc);
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
