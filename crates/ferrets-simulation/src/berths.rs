//! The berths a job offers attached workers: which are free, where each one
//! is on the map, and the movement of workers at and between them.

use std::cmp::Reverse;

use bevy_ecs::{entity::Entity, query::Without, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::{
    FixedI64, FixedU64,
    facing::{self, Facing},
    fixed_uvec2::FixedUVec2,
    fixed_vec2::FixedVec2,
};
use ferrets_physics::body;
use ferrets_random::hash;

use crate::{
    components::{
        attached::{AttachedComponent, Sitting, Way},
        berths::BerthsComponent,
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        location::LocationComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    simulation_id::SimulationId,
};
use ferrets_content::work::{BerthStance, WorkPresence};

/// Whether `job` offers the berth group `group` at all.
pub fn offers(world: &World, job: Entity, group: &str) -> bool {
    entity_def::of(world, job)
        .berths
        .as_ref()
        .is_some_and(|berths| berths.group(group).is_some())
}

/// Whether `job` shuts a newcomer of `presence` out: the newcomer attaches,
/// and the group it sits in is not offered or has no free berth, or the job's
/// form is changing. A newcomer that does not attach is never shut out here.
pub fn shut(world: &World, job: Entity, presence: &WorkPresence) -> bool {
    match presence.attachment() {
        Some(attachment) => {
            !offers(world, job, attachment.berths())
                || free_seats(world, job, attachment.berths()).is_empty()
                || entity_def::changing(world, job)
        }
        None => false,
    }
}

/// Whether anyone sits in a berth of `job` other than the entities in
/// `except`.
pub fn seated_by_others(world: &World, job: Entity, except: &[SimulationId]) -> bool {
    world
        .entity(job)
        .get::<BerthsComponent>()
        .is_some_and(|berths| {
            berths
                .seats
                .values()
                .flatten()
                .flatten()
                .any(|holder| !except.contains(holder))
        })
}

/// The free berth of `group` on `job` nearest to `from`, as its index and its
/// point, or `None` when the group is full or not offered. Ties go to the
/// lowest index.
pub fn nearest_free_seat(
    world: &World,
    job: Entity,
    group: &str,
    from: CellRect,
) -> Option<(usize, FixedUVec2)> {
    let projection = world.resource::<Map>().projection();
    free_seats(world, job, group)
        .into_iter()
        .map(|seat| (seat, seat_point(world, job, group, seat)))
        .min_by_key(|&(_, point)| projection.rect_distance(CellPos::from(point), from))
}

/// Seats `worker` in berth `seat` of `group` on `job`.
///
/// Panics if the berth is taken: a caller seats a worker only in a berth it
/// just found free.
pub fn occupy(world: &mut World, job: Entity, group: &str, seat: usize, worker: SimulationId) {
    let points = group_len(world, job, group);
    let mut job_mut = world.entity_mut(job);
    let mut berths = match job_mut.get_mut::<BerthsComponent>() {
        Some(berths) => berths,
        None => {
            job_mut.insert(BerthsComponent::default());
            job_mut
                .get_mut::<BerthsComponent>()
                .expect("inserted above")
        }
    };
    let seats = berths
        .seats
        .entry(group.to_string())
        .or_insert_with(|| vec![None; points]);
    assert!(
        seats[seat].is_none(),
        "a worker is seated only in a berth found free"
    );
    seats[seat] = Some(worker);
}

/// Frees the berth an attached worker holds, as its `attached` marker records
/// it.
/// A job that has already left the world has no berths left to free.
///
/// Panics if the job does not seat the worker: a berth is given up once, by
/// the worker holding it.
pub fn vacate(world: &mut World, attached: &AttachedComponent) {
    let Some(job) = world.resource::<EntityIndex>().any(attached.job) else {
        return;
    };
    let mut job_mut = world.entity_mut(job);
    let mut berths = job_mut
        .get_mut::<BerthsComponent>()
        .expect("a berth is given up only on a job seating its holder");
    let seats = berths
        .seats
        .get_mut(&attached.berths)
        .expect("a berth is given up only in a group laid out for its holder");
    seats[attached.seat] = None;
    let empty = berths
        .seats
        .values()
        .all(|seats: &Vec<Option<SimulationId>>| seats.iter().all(Option::is_none));
    if empty {
        job_mut.remove::<BerthsComponent>();
    }
}

/// The point berth `seat` of `group` on `job` is at — where a worker's
/// middle sits. A point that would lie off the map is held at the middle of
/// the map's edge cell.
fn seat_point(world: &World, job: Entity, group: &str, seat: usize) -> FixedUVec2 {
    let anchor = FixedUVec2::from(body::anchor(entity_def::position(world, job)));
    let point = unsigned(signed(anchor) + group_points(world, job, group)[seat]);
    let map = world.resource::<Map>();
    let half = FixedU64::ONE / 2;
    FixedUVec2::new(
        point.x.min(FixedU64::from_num(map.width()) - half),
        point.y.min(FixedU64::from_num(map.height()) - half),
    )
}

/// The position that puts `worker`'s middle on `point`: half the worker's
/// footprint comes off the point, since a position names the corner its
/// footprint starts at.
pub fn position_centred_on(world: &World, worker: Entity, point: FixedUVec2) -> FixedUVec2 {
    let size = entity_def::of(world, worker)
        .location
        .expect("validated content defines a location")
        .size();
    let half = |cells: u32| FixedU64::from_num(cells) / 2;
    FixedUVec2::new(
        point.x.saturating_sub(half(size.width)),
        point.y.saturating_sub(half(size.height)),
    )
}

/// How `worker`, of `stance`, sits down as it takes its first berth on a job.
///
/// Workers seated in the same tick do not then move in step: a circling
/// worker's first stay is already part way along, and an orbiting worker's
/// circuit starts part way round, both by an amount mixed from the worker's
/// id (see [`hash::mix`]).
pub fn sit_down(stance: BerthStance, worker: SimulationId) -> Sitting {
    match stance {
        BerthStance::Still => Sitting::Seated { dwelt: 0 },
        BerthStance::Circling { dwell, .. } | BerthStance::Roaming { dwell, .. } => {
            Sitting::Seated {
                dwelt: if dwell == 0 {
                    0
                } else {
                    hash::mix(worker.0, 0) % dwell
                },
            }
        }
        BerthStance::Orbit { .. } => Sitting::Orbiting {
            phase: Facing::from_bits(
                ((hash::mix(worker.0, 0) % 4) * (facing::PER_TURN / 4)) as u16,
            ),
        },
    }
}

/// Moves every attached worker one tick along its stance, in ascending
/// simulation-id order.
///
/// A circling worker that has worked its berth long enough picks a way round
/// the group's loop by a mix of its id and how often it has moved, takes the
/// free berth that way farthest from every other seated worker — the nearest
/// ahead among equals — and walks the loop to it. A roaming one takes a free
/// berth picked by the same mix and crosses straight to it. Both arrive to
/// work again. An orbiting worker turns further round its berth. A still
/// worker never moves, and neither does one with no free berth to move to, or
/// one that is dying.
pub fn advance(world: &mut World) {
    let mut moving: Vec<(SimulationId, Entity)> = world
        .query_filtered::<(Entity, &EntityInfoComponent, &AttachedComponent), Without<DyingComponent>>()
        .iter(world)
        .filter(|(_, _, attached)| match attached.stance {
            BerthStance::Still => false,
            BerthStance::Circling { .. }
            | BerthStance::Roaming { .. }
            | BerthStance::Orbit { .. } => true,
        })
        .map(|(entity, info, _)| (info.id(), entity))
        .collect();
    moving.sort_unstable_by_key(|&(id, _)| id);

    for (id, entity) in moving {
        let attached = world
            .entity(entity)
            .get::<AttachedComponent>()
            .expect("collected above")
            .clone();
        let Some(job) = world.resource::<EntityIndex>().alive(attached.job) else {
            continue;
        };
        // Where the worker's middle goes this tick, if it moves.
        let mut hops = attached.hops;
        let (sitting, seat, position) = match (attached.stance, attached.sitting) {
            (BerthStance::Still, _) => unreachable!("collected only moving workers"),
            (
                stance @ (BerthStance::Circling { dwell, .. } | BerthStance::Roaming { dwell, .. }),
                Sitting::Seated { dwelt },
            ) => {
                let dwelt = dwelt.saturating_add(1);
                if dwelt < dwell {
                    (Sitting::Seated { dwelt }, attached.seat, None)
                } else {
                    // The berth to move to, if any is free: the next round the
                    // loop for a circling worker, one picked by the worker's
                    // own mix for a roaming one, so neighbours who sat down
                    // together part ways. The berth held passes to whoever
                    // comes next as this worker leaves.
                    // A seated worker moving on keeps its slot, so only the
                    // points count here, not the slot cap.
                    let free = free_points(world, job, &attached.berths);
                    // A circling worker also picks which way round to walk,
                    // by the same mix.
                    let way = if hash::mix(id.0, hops).is_multiple_of(2) {
                        Way::Forward
                    } else {
                        Way::Backward
                    };
                    let next = match stance {
                        BerthStance::Circling { .. } => {
                            spread_pick(world, job, &attached.berths, attached.seat, &free, way)
                        }
                        BerthStance::Roaming { .. } => (!free.is_empty())
                            .then(|| free[hash::mix(id.0, hops) as usize % free.len()]),
                        BerthStance::Still | BerthStance::Orbit { .. } => {
                            unreachable!("matched a stance that moves between berths")
                        }
                    };
                    match next {
                        Some(next) => {
                            vacate(world, &attached);
                            occupy(world, job, &attached.berths, next, id);
                            hops += 1;
                            (
                                Sitting::Travelling {
                                    from: attached.seat,
                                    along: FixedU64::ZERO,
                                    way,
                                },
                                next,
                                None,
                            )
                        }
                        None => (Sitting::Seated { dwelt }, attached.seat, None),
                    }
                }
            }
            (
                stance @ (BerthStance::Circling { speed, .. } | BerthStance::Roaming { speed, .. }),
                Sitting::Travelling { from, along, way },
            ) => {
                let along = along + speed;
                let point = match stance {
                    BerthStance::Circling { .. } => point_along_loop(
                        world,
                        job,
                        &attached.berths,
                        from,
                        attached.seat,
                        way,
                        along,
                    ),
                    BerthStance::Roaming { .. } => {
                        point_between(world, job, &attached.berths, from, attached.seat, along)
                    }
                    BerthStance::Still | BerthStance::Orbit { .. } => {
                        unreachable!("matched a stance that moves between berths")
                    }
                };
                match point {
                    Some(point) => (
                        Sitting::Travelling { from, along, way },
                        attached.seat,
                        Some(point),
                    ),
                    None => (
                        Sitting::Seated { dwelt: 0 },
                        attached.seat,
                        Some(seat_point(world, job, &attached.berths, attached.seat)),
                    ),
                }
            }
            (BerthStance::Orbit { radius, period }, Sitting::Orbiting { phase }) => {
                let step = (facing::PER_TURN / period) % facing::PER_TURN;
                let phase = Facing::from_bits(phase.to_bits().wrapping_add(step as u16));
                let centre = seat_point(world, job, &attached.berths, attached.seat);
                (
                    Sitting::Orbiting { phase },
                    attached.seat,
                    Some(offset(centre, phase.unit() * radius.to_num::<FixedI64>())),
                )
            }
            (
                BerthStance::Circling { .. } | BerthStance::Roaming { .. },
                Sitting::Orbiting { .. },
            )
            | (BerthStance::Orbit { .. }, Sitting::Seated { .. } | Sitting::Travelling { .. }) => {
                unreachable!("a worker sits the way its stance says")
            }
        };

        let position = position.map(|point| position_centred_on(world, entity, point));
        let mut entity_mut = world.entity_mut(entity);
        if let Some(position) = position {
            entity_mut
                .get_mut::<LocationComponent>()
                .expect("an attached worker has a location")
                .position = position;
        }
        let mut attached = entity_mut
            .get_mut::<AttachedComponent>()
            .expect("collected above");
        attached.sitting = sitting;
        attached.seat = seat;
        attached.hops = hops;
    }
}

/// The free point of `group` a circling worker leaving `seat` and walking
/// `way` heads for: the one whose nearest other seated worker is most steps
/// away round the loop, and among those the fewest steps ahead its way.
/// `None` when nothing is free.
fn spread_pick(
    world: &World,
    job: Entity,
    group: &str,
    seat: usize,
    free: &[usize],
    way: Way,
) -> Option<usize> {
    let count = group_len(world, job, group);
    let others: Vec<usize> = world
        .entity(job)
        .get::<BerthsComponent>()
        .and_then(|seats| seats.seats.get(group))
        .map(|seats| {
            seats
                .iter()
                .enumerate()
                .filter(|&(point, holder)| point != seat && holder.is_some())
                .map(|(point, _)| point)
                .collect()
        })
        .unwrap_or_default();
    // Steps round the loop between two points, the short way.
    let apart = |a: usize, b: usize| {
        let ahead = (b + count - a) % count;
        ahead.min(count - ahead)
    };
    // Candidates in order of steps ahead its way; the first among equals is
    // the nearest ahead, which is why the best is found as the least of a
    // reversed score.
    (1..count)
        .map(|step| step_round(seat, step, count, way))
        .filter(|point| free.contains(point))
        .min_by_key(|&point| {
            // The nearest neighbour decides; a loop with nobody else on it
            // makes every point as good.
            Reverse(
                others
                    .iter()
                    .map(|&other| apart(point, other))
                    .min()
                    .unwrap_or(count),
            )
        })
}

/// The point `step` berths from `seat` round a loop of `count` berths, going
/// `way`.
fn step_round(seat: usize, step: usize, count: usize, way: Way) -> usize {
    match way {
        Way::Forward => (seat + step) % count,
        Way::Backward => (seat + count - step % count) % count,
    }
}

/// The point `along` cells round the group's loop — its berths in declared
/// order, walked `way` — from berth `from` toward berth `to`, or `None` once
/// that distance reaches `to`.
fn point_along_loop(
    world: &World,
    job: Entity,
    group: &str,
    from: usize,
    to: usize,
    way: Way,
    mut along: FixedU64,
) -> Option<FixedUVec2> {
    let projection = world.resource::<Map>().projection();
    let points = group_len(world, job, group);
    let mut at = from;
    while at != to {
        let next = step_round(at, 1, points, way);
        let (a, b) = (
            seat_point(world, job, group, at),
            seat_point(world, job, group, next),
        );
        let stretch = projection.span(a.x.abs_diff(b.x), a.y.abs_diff(b.y));
        if along < stretch {
            let fraction = (along / stretch).to_num::<FixedI64>();
            let (a, b) = (signed(a), signed(b));
            return Some(unsigned(a + (b - a) * fraction));
        }
        along -= stretch;
        at = next;
    }
    None
}

/// The point `along` cells down the straight line from berth `from` to berth
/// `to`, or `None` once that distance reaches `to`.
fn point_between(
    world: &World,
    job: Entity,
    group: &str,
    from: usize,
    to: usize,
    along: FixedU64,
) -> Option<FixedUVec2> {
    let projection = world.resource::<Map>().projection();
    let (a, b) = (
        seat_point(world, job, group, from),
        seat_point(world, job, group, to),
    );
    let stretch = projection.span(a.x.abs_diff(b.x), a.y.abs_diff(b.y));
    if along >= stretch {
        return None;
    }
    let fraction = (along / stretch).to_num::<FixedI64>();
    let (a, b) = (signed(a), signed(b));
    Some(unsigned(a + (b - a) * fraction))
}

/// The berths of `group` on `job` open to a newcomer, by index: the points
/// nobody holds — or none at all once the group seats as many workers as it
/// has slots. Empty when the group is not offered.
fn free_seats(world: &World, job: Entity, group: &str) -> Vec<usize> {
    let Some(berths) = entity_def::of(world, job)
        .berths
        .as_ref()
        .and_then(|berths| berths.group(group))
    else {
        return Vec::new();
    };
    let taken = world
        .entity(job)
        .get::<BerthsComponent>()
        .and_then(|seats| seats.seats.get(group));
    let seated = taken.map_or(0, |seats| seats.iter().flatten().count());
    if seated >= berths.slots() {
        return Vec::new();
    }
    free_points(world, job, group)
}

/// The points of `group` on `job` nobody holds, by index, whatever the group's
/// slots say — what a seated worker moving on may pick from, since it keeps
/// its slot.
///
/// Panics if the job does not offer the group.
fn free_points(world: &World, job: Entity, group: &str) -> Vec<usize> {
    let taken = world
        .entity(job)
        .get::<BerthsComponent>()
        .and_then(|seats| seats.seats.get(group));
    (0..group_len(world, job, group))
        .filter(|&seat| taken.is_none_or(|seats| seats[seat].is_none()))
        .collect()
}

/// The berth points of `group` on `job`, as signed offsets from the
/// footprint's anchor.
///
/// Panics if the job does not offer the group.
fn group_points<'a>(world: &'a World, job: Entity, group: &str) -> &'a [FixedVec2] {
    entity_def::of(world, job)
        .berths
        .as_ref()
        .and_then(|berths| berths.group(group))
        .expect("a berth is read only on a job offering its group")
        .points()
}

/// How many berths `group` on `job` has.
///
/// Panics if the job does not offer the group.
fn group_len(world: &World, job: Entity, group: &str) -> usize {
    group_points(world, job, group).len()
}

/// `centre` moved by `delta`, never before the map's first cell.
fn offset(centre: FixedUVec2, delta: FixedVec2) -> FixedUVec2 {
    unsigned(signed(centre) + delta)
}

/// `point` as a signed vector.
fn signed(point: FixedUVec2) -> FixedVec2 {
    FixedVec2::new(point.x.to_num::<FixedI64>(), point.y.to_num::<FixedI64>())
}

/// `point` as an unsigned vector; a coordinate below zero comes back as zero.
fn unsigned(point: FixedVec2) -> FixedUVec2 {
    FixedUVec2::new(
        point.x.max(FixedI64::ZERO).to_num::<FixedU64>(),
        point.y.max(FixedI64::ZERO).to_num::<FixedU64>(),
    )
}
