//! Shared chase logic for orders that walk toward a destination footprint.
//!
//! Orders that close on a destination do it the same way: arrive if already in
//! range, otherwise walk toward it, otherwise give up when the previous walk made
//! no progress.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{astar, astar::Projection, nav_pos::NavPos, nav_size::NavSize};

use crate::{components::location::LocationComponent, entity_def, map::Map, order::Order};

/// Where a chaser stands relative to its destination this tick.
pub enum Destination {
    /// Within range of the destination footprint.
    Arrived,
    /// A walk toward the destination should be issued as a sub-order.
    Walk(Order),
    /// The previous walk made no progress; the destination is unreachable.
    OutOfReach,
}

/// `(chaser position, destination position)` recorded when the last chase move
/// started. When both are unchanged on resume the chase made no progress and
/// never will. For a stationary destination only the chaser position can change,
/// so this reduces to tracking the chaser; for a moving destination it gives up
/// only when neither has moved.
pub type ChaseState = Option<(FixedUVec2, FixedUVec2)>;

/// Decides whether `from` is within `range` of the footprint at `destination`
/// with `destination_size`, and otherwise whether to walk toward it or give up.
pub fn advance(
    last_chase: &mut ChaseState,
    projection: Projection,
    from: FixedUVec2,
    destination: FixedUVec2,
    destination_size: NavSize,
    range: u32,
) -> Destination {
    if astar::in_range_of_rect(
        projection,
        NavPos::from(from),
        NavPos::from(destination),
        destination_size,
        range,
    ) {
        *last_chase = None;
        return Destination::Arrived;
    }

    let progress = (from, destination);
    if *last_chase == Some(progress) {
        return Destination::OutOfReach;
    }
    *last_chase = Some(progress);
    Destination::Walk(Order::Move {
        target: destination,
        size: destination_size,
        range,
    })
}

/// Turns `entity` to look at the footprint at `target`/`target_size`, from its own
/// middle toward the nearest part of that footprint.
///
/// A position names a footprint's first cell, so aiming at the position itself
/// turns a unit toward one corner of a keep rather than at the keep. Aiming at the
/// middle is no better for a long wall — it would face along the wall instead of at
/// the stretch in front of it. The nearest point is both, and it is the same
/// measure every range check already uses.
///
/// A no-op while the unit's middle is inside the footprint, so the previous facing
/// is kept rather than zeroed. Orders call this once in range so a unit faces what
/// it acts on.
pub fn face(world: &mut World, entity: Entity, target: FixedUVec2, target_size: NavSize) {
    let middle = center_of(entity_def::footprint(world, entity));

    let facing = nearest_point_on(target, target_size, middle) - middle;
    if facing != FixedVec2::ZERO {
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .expect("a chasing entity has a location")
            .facing = facing;
    }
}

/// Like [`face`], with the footprint taken from another entity.
pub fn face_entity(world: &mut World, entity: Entity, target: Entity) {
    let (target_position, target_size) = entity_def::footprint(world, target);
    face(world, entity, target_position, target_size);
}

/// The point of the footprint at `origin`/`size` closest to `from`, clamping each
/// axis into the footprint's span. Continuous rather than per-cell, so a unit part
/// way across a cell turns smoothly instead of in steps.
fn nearest_point_on(origin: FixedUVec2, size: NavSize, from: FixedVec2) -> FixedVec2 {
    let span = |start: FixedI64, cells: u32, value: FixedI64| {
        value.clamp(start, start + FixedI64::from_num(cells))
    };
    FixedVec2::new(
        span(origin.x.to_num::<FixedI64>(), size.width, from.x),
        span(origin.y.to_num::<FixedI64>(), size.height, from.y),
    )
}

/// Like [`advance`], with the destination taken from a target entity's location.
pub fn advance_to_entity(
    last_chase: &mut ChaseState,
    world: &World,
    from: FixedUVec2,
    destination: Entity,
    range: u32,
) -> Destination {
    let (destination_position, destination_size) = entity_def::footprint(world, destination);
    let projection = world.resource::<Map>().projection();

    advance(
        last_chase,
        projection,
        from,
        destination_position,
        destination_size,
        range,
    )
}

/// The middle of a footprint, in world units.
fn center_of((origin, size): (FixedUVec2, NavSize)) -> FixedVec2 {
    let half = |cells: u32| FixedI64::from_num(cells) / 2;
    FixedVec2::new(
        origin.x.to_num::<FixedI64>() + half(size.width),
        origin.y.to_num::<FixedI64>() + half(size.height),
    )
}
