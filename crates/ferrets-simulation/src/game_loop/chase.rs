//! Shared chase logic for orders that walk toward a destination footprint.
//!
//! Attack, follow, build, and harvest all close on a destination the same way:
//! arrive if already in range, otherwise walk toward it, otherwise give up when
//! the previous walk made no progress.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::{astar, astar::Projection, nav_pos::NavPos, nav_size::NavSize};

use crate::{
    components::location::{LocationComponent, LocationStaticData},
    map::Map,
    order::Order,
};

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
        range,
    })
}

/// Like [`advance`], with the destination taken from a target entity's location.
pub fn advance_to_entity(
    last_chase: &mut ChaseState,
    world: &World,
    from: FixedUVec2,
    destination: Entity,
    range: u32,
) -> Destination {
    let destination_position = world
        .entity(destination)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let destination_size = world
        .entity(destination)
        .get::<LocationStaticData>()
        .unwrap()
        .size();
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
