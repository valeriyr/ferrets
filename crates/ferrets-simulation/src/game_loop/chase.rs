//! Shared chase logic for orders that walk toward a destination footprint.
//!
//! Orders that close on a destination do it the same way: arrive if already in
//! range, otherwise walk toward it, otherwise give up when the previous walk made
//! no progress.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_physics::body;

use crate::{
    components::chase::{ChaseRound, ChaseState},
    components::location::LocationComponent,
    entity_def,
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

/// Decides whether `from` is within `range` of the footprint at `destination`
/// with `destination_size`, and otherwise whether to walk toward it or give up.
pub fn advance(
    last_chase: &mut ChaseState,
    projection: Projection,
    from: FixedUVec2,
    from_size: CellSize,
    destination: FixedUVec2,
    destination_size: CellSize,
    range: u32,
) -> Destination {
    // The chaser judges itself by the cell under its body's center — the
    // cell it visually stands on and claims. A continuous body pressed
    // against its destination rests with a fractional anchor whose floor
    // is one cell short, and floor-judging it would re-walk that phantom
    // step forever. The anchor's own floor also counts: a body nudged off
    // its arrival spot by a working neighbor straddles the boundary with
    // its center a hair past it, still standing at the work for any eye —
    // and re-walking that hair loses the spot-contest forever. The
    // destination stays anchor-floored: that is the cell the walk below
    // plans toward and accepts by, so both verdicts name the same spot.
    //
    // Both sides are footprints: a wide chaser reaches as far as its nearest
    // edge, which is also how acquisition measures — the two must agree, or a
    // wide unit would acquire a target it then walks straight past.
    let goal = CellRect::new(CellPos::from(destination), destination_size);
    let standing = CellRect::new(body::anchor(from), from_size);
    let floored = CellRect::new(CellPos::from(from), from_size);
    if projection.in_range_for_rects(standing, goal, range)
        || projection.in_range_for_rects(floored, goal, range)
    {
        *last_chase = None;
        return Destination::Arrived;
    }

    let (own, destination_cell) = (body::anchor(from), CellPos::from(destination));
    match last_chase {
        Some(round) if (round.own, round.destination) == (own, destination_cell) => {
            round.repeats += 1;
            if round.exhausted() {
                return Destination::OutOfReach;
            }
        }
        _ => {
            *last_chase = Some(ChaseRound {
                own,
                destination: destination_cell,
                repeats: 0,
            });
        }
    }
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
pub fn face(world: &mut World, entity: Entity, target: FixedUVec2, target_size: CellSize) {
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
fn nearest_point_on(origin: FixedUVec2, size: CellSize, from: FixedVec2) -> FixedVec2 {
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
    chaser: Entity,
    destination: Entity,
    range: u32,
) -> Destination {
    let (from, from_size) = entity_def::footprint(world, chaser);
    let (destination_position, destination_size) = entity_def::footprint(world, destination);
    let projection = world.resource::<Map>().projection();

    advance(
        last_chase,
        projection,
        from,
        from_size,
        destination_position,
        destination_size,
        range,
    )
}

/// The middle of a footprint, in world units.
fn center_of((origin, size): (FixedUVec2, CellSize)) -> FixedVec2 {
    let half = |cells: u32| FixedI64::from_num(cells) / 2;
    FixedVec2::new(
        origin.x.to_num::<FixedI64>() + half(size.width),
        origin.y.to_num::<FixedI64>() + half(size.height),
    )
}
