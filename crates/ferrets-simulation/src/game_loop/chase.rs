//! Shared chase logic for orders that walk toward a destination.
//!
//! An order that closes on a footprint or a cell does it the same way: arrive
//! if already in range, otherwise walk toward it, otherwise give up when the
//! previous walk made no progress. One that closes on a point walks once.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{
    FixedI64, FixedU64, facing::Facing, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2,
    fixed_vec2::FixedVec2,
};

use super::{orders, turn};
use crate::{
    components::chase::{ChaseRound, ChaseState},
    entity_def,
    map::Map,
    movement_model,
    order::Order,
    reach,
};

/// Where a chaser stands relative to its destination this tick.
pub enum Destination {
    /// At the destination: within range of its footprint, or on its point.
    Arrived,
    /// A walk toward the destination should be issued as a sub-order.
    Walk(Order),
    /// The destination is unreachable: the previous walk made no progress, or
    /// the chaser is held in a berth and walks nowhere.
    OutOfReach,
}

/// Decides whether `chaser` is within `range` of the cells of `goal`, and
/// otherwise whether to walk toward it or give up.
pub fn advance_to_rect(
    last_chase: &mut ChaseState,
    world: &World,
    chaser: Entity,
    goal: CellRect,
    range: u32,
) -> Destination {
    close_on(last_chase, world, chaser, goal, range)
}

/// Like [`advance_to_rect`], toward the target entity `destination`, a
/// body reached at every cell it stands on ([`entity_def::standing_rect`]).
pub fn advance_to_entity(
    last_chase: &mut ChaseState,
    world: &World,
    chaser: Entity,
    destination: Entity,
    range: u32,
) -> Destination {
    let goal = entity_def::standing_rect(world, destination);
    close_on(last_chase, world, chaser, goal, range)
}

/// Decides whether `chaser` stands lined up anchored at `cell`, and otherwise
/// whether to walk there or give up.
///
/// Panics if `chaser`'s body anchored at `cell` would not lie wholly on the
/// map: the cell is one its body can be anchored at.
pub fn advance_onto_cell(
    last_chase: &mut ChaseState,
    world: &World,
    chaser: Entity,
    cell: CellPos,
) -> Destination {
    let standing = entity_def::location(world, chaser);
    assert_eq!(
        world
            .resource::<Map>()
            .location_at_clamped(cell, standing.size())
            .anchor(),
        cell,
        "a cell a body is walked onto is one its body can be anchored at"
    );
    if !standing.is_mid_crossing() && standing.anchor() == cell {
        *last_chase = None;
        return Destination::Arrived;
    }
    if entity_def::held(world, chaser) {
        return Destination::OutOfReach;
    }
    let walk = orders::move_onto_cell(world, chaser, cell);
    walk_or_give_up(last_chase, world, chaser, cell, walk)
}

/// Decides whether `chaser` stands where a point move to `target` leaves it
/// ([`movement_model::at_point`]), and otherwise whether it walks there: once,
/// the walk's end being the caller's, so no round is counted.
pub fn advance_to_point(world: &World, chaser: Entity, target: FixedUVec2) -> Destination {
    if movement_model::at_point(world, chaser, target) {
        return Destination::Arrived;
    }
    if entity_def::held(world, chaser) {
        return Destination::OutOfReach;
    }
    Destination::Walk(orders::move_to_point(target))
}

/// Turns `entity` to look at `target`, from its own middle toward the nearest
/// point of `target`.
///
/// A no-op while the unit's middle is inside `target`: the previous look is
/// kept.
///
/// Standing, so the body comes round at the rate it turns on the spot. A weapon
/// that bears on its own is aimed instead of faced — see [`super::turret`].
pub fn face(world: &mut World, entity: Entity, target: FixedURect) {
    if let Some(wanted) = bearing_to(world, entity, target) {
        turn::toward(world, entity, wanted, turn::Rate::Standing);
    }
}

/// Like [`face`], toward the bounds of another entity.
pub fn face_entity(world: &mut World, entity: Entity, target: Entity) {
    let bounds = entity_def::location(world, target).bounds();
    face(world, entity, bounds);
}

/// The bearing from `entity`'s own middle to the nearest point of `target`,
/// or `None` while its middle is inside `target` — where there is no
/// direction to point.
pub fn bearing_to(world: &World, entity: Entity, target: FixedURect) -> Option<Facing> {
    let position = entity_def::position(world, entity);
    let middle = FixedVec2::new(
        position.x.to_num::<FixedI64>(),
        position.y.to_num::<FixedI64>(),
    );
    Facing::of(nearest_point_on(target, middle) - middle)
}

/// The point of `bounds` closest to `from`, clamping each axis into its span.
/// Continuous rather than per-cell, so a unit part way across a cell turns
/// smoothly instead of in steps.
fn nearest_point_on(bounds: FixedURect, from: FixedVec2) -> FixedVec2 {
    let span = |low: FixedU64, high: FixedU64, value: FixedI64| {
        value.clamp(low.to_num::<FixedI64>(), high.to_num::<FixedI64>())
    };
    FixedVec2::new(
        span(bounds.min().x, bounds.max().x, from.x),
        span(bounds.min().y, bounds.max().y, from.y),
    )
}

/// Arrives when `chaser` reaches `goal` within `range`, else walks to within
/// `range` of `goal`, or gives up when the last walk made no progress.
fn close_on(
    last_chase: &mut ChaseState,
    world: &World,
    chaser: Entity,
    goal: CellRect,
    range: u32,
) -> Destination {
    // The chaser is judged by every cell it stands on. Judging a body by one
    // cell of the two it lies across leaves it a little past a corner reading
    // as out of range on one axis or the other however it is quantized, and a
    // working neighbor's nudge decides which — so the walk arrives, the nudge
    // lands, this says out of range, and the round counts as no progress until
    // the whole order gives up. The walk below plans toward the goal itself,
    // so a walk arrives by its own measure exactly where it arrives by this
    // one.
    //
    // Both sides are footprints: a wide chaser reaches as far as its nearest
    // edge, which is also how acquisition measures — the two must agree, or a
    // wide unit would acquire a target it then walks straight past.
    if reach::in_reach(world, chaser, goal, range) {
        *last_chase = None;
        return Destination::Arrived;
    }
    if entity_def::held(world, chaser) {
        return Destination::OutOfReach;
    }

    // A walk onto a single cell brings the chaser anchored there; any other
    // goal is named by its first cell.
    let walk = match (range, goal.size == CellSize::ONE) {
        (0, true) => orders::move_onto_cell(world, chaser, goal.origin),
        _ => Order::Move {
            target: goal.origin.middle(),
            size: goal.size,
            range,
        },
    };
    walk_or_give_up(last_chase, world, chaser, goal.origin, walk)
}

/// Sends `chaser` on `walk` toward `destination`, or gives up when the last
/// walk toward the same destination from the same cell made no progress.
fn walk_or_give_up(
    last_chase: &mut ChaseState,
    world: &World,
    chaser: Entity,
    destination: CellPos,
    walk: Order,
) -> Destination {
    let own = entity_def::location(world, chaser).anchor();
    match last_chase {
        Some(round) if (round.own, round.destination) == (own, destination) => {
            round.repeats += 1;
            if round.exhausted() {
                return Destination::OutOfReach;
            }
        }
        _ => {
            *last_chase = Some(ChaseRound {
                own,
                destination,
                repeats: 0,
            });
        }
    }
    Destination::Walk(walk)
}
