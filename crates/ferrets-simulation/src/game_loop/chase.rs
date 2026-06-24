//! Shared chase logic for orders that walk toward a destination footprint.
//!
//! Orders that close on a destination do it the same way: arrive if already in
//! range, otherwise walk toward it, otherwise give up when the previous walk made
//! no progress.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
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

/// Turns `entity` to look toward the `target` cell from its current position.
/// A no-op when they share a cell, so the previous facing is kept rather than
/// zeroed. Orders call this once in range so a unit faces what it acts on.
pub fn face(world: &mut World, entity: Entity, target: FixedUVec2) {
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    let facing = FixedVec2::new(
        target.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
        target.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
    );
    if facing != FixedVec2::ZERO {
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .unwrap()
            .facing = facing;
    }
}

/// Like [`face`], with the target taken from another entity's location.
pub fn face_entity(world: &mut World, entity: Entity, target: Entity) {
    let target_position = world
        .entity(target)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    face(world, entity, target_position);
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
