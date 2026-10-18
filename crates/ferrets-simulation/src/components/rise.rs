//! The state of an entity still rising into the world, and how its pools
//! follow the work of its rise.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use crate::{
    components::pools::{self, Follows, PoolsComponent},
    entity_def,
    simulation_id::SimulationId,
};
use ferrets_content::{
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    registry::ContentRegistry,
    rise::RisePool,
};

/// How a rising entity's progress is advanced: by the orders of the workers
/// on it, by the entity itself once whatever brought it in left it
/// unattended, or not at all while it waits for a worker to take it up
/// again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiseAttendance {
    /// Workers are on the entity, hidden inside it, on it or beside it, and
    /// their orders advance the progress, one tick per tick each. The entity
    /// never advances on its own.
    Crew {
        /// The workers whose orders are on the entity right now.
        workers: BTreeSet<SimulationId>,
    },
    /// No worker's order is on the entity; it advances itself one tick per
    /// tick, and takes no crew.
    Unattended {
        /// The entity the rise advances on behalf of: the one that brought it
        /// into the world, or one that has since taken it up.
        founder: SimulationId,
    },
    /// No worker's order is on the entity and it does not advance: its crew
    /// left it with the progress put in so far, and any worker that can take
    /// up the type takes it up again.
    Halted,
}

/// Marks an entity still rising into the world, holding the progress of its
/// rise: several workers advance the same work, and what they have put in
/// outlives the one that started it.
#[derive(Component, Debug)]
pub struct RisingComponent {
    /// Ticks of work put into the rise.
    pub progress: u32,
    /// How the progress is advanced, and on whose behalf.
    pub attendance: RiseAttendance,
}

impl RisingComponent {
    /// A rise with no work put in yet, attended as `attendance` says.
    pub fn new(attendance: RiseAttendance) -> Self {
        Self {
            progress: 0,
            attendance,
        }
    }

    /// Steps the attendance for the `tender` offering to advance the rise: a
    /// halted rise is taken up as that tender's own, an unattended one whose
    /// tender has gone is halted again, and one with a crew on it is being
    /// worked by that crew already.
    pub fn tend(&mut self, tender: Option<SimulationId>) {
        match (&self.attendance, tender) {
            (RiseAttendance::Halted, Some(founder)) => {
                self.attendance = RiseAttendance::Unattended { founder }
            }
            (RiseAttendance::Unattended { .. }, None) => self.attendance = RiseAttendance::Halted,
            (RiseAttendance::Crew { .. }, _)
            | (RiseAttendance::Halted, None)
            | (RiseAttendance::Unattended { .. }, Some(_)) => {}
        }
    }
}

/// Moves each pool of the rising `entity` that follows its work by how far
/// the work's line moved since last read: the line stands at `start +
/// (maximum − start) × progress ÷ rise time`, `start` being the pool's rise
/// initial under the maximum, both read as they stand now, so what the
/// entity lost or gained besides stays as it was. Held between empty and the
/// maximum; a live health pool is never emptied by it. Panics when `entity`
/// is not rising with a pool following its work, or its work is past its
/// rise time.
pub fn rise(world: &mut World, entity: Entity) {
    let progress = world
        .entity(entity)
        .get::<RisingComponent>()
        .expect("rise is given a rising entity")
        .progress;
    let store = world
        .get::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store");
    let following: Vec<(PoolId, PoolInitial, FixedU64, FixedU64)> = rise_pools(world, entity)
        .into_iter()
        .filter_map(
            |(declared, holding)| match (holding, store.follows(declared.id())) {
                (RisePool::FollowsWork(initial), Some(Follows::Work { line })) => Some((
                    declared.id(),
                    PoolInitial::from(initial),
                    line,
                    store
                        .current(declared.id())
                        .expect("a pool following work is held"),
                )),
                (RisePool::FollowsWork(_), Some(Follows::Maximum) | None) => {
                    unreachable!(
                        "a rising entity's pool follows its work: {:?}",
                        declared.id()
                    )
                }
                (RisePool::Initial | RisePool::GainedOnRisen, _) => None,
            },
        )
        .collect();
    assert!(
        !following.is_empty(),
        "rise is given a rising entity with a pool following its work"
    );
    let rise_time = entity_def::of(world, entity)
        .rise_time()
        .expect("a rising entity with a pool following its work is of a type that rises");
    assert!(
        progress <= rise_time,
        "a rise's work stops at its rise time: {progress} > {rise_time}"
    );
    world.resource_scope(|world, registry: Mut<ContentRegistry>| {
        for (pool, initial, last, current) in following {
            let maximum =
                entity_def::effective_stat(world, entity, registry.pool_def(pool).maximum_stat())
                    .expect("a rising entity carries the maximum of each pool following its work");
            let start = initial.under(maximum);
            // Widened so the product neither overflows nor rounds before the
            // division; at most `maximum − start`, it narrows back exactly.
            let gained = u128::from((maximum - start).to_bits()) * u128::from(progress)
                / u128::from(rise_time);
            let line = start
                + FixedU64::from_bits(u64::try_from(gained).expect("the gain is within the span"));
            step_to_line(
                world,
                &registry,
                entity,
                pool,
                (last, current),
                line,
                maximum,
            );
        }
    });
}

/// Marks the rising `entity` as risen: takes its rising state off and gives
/// it each pool its type gains on risen at that pool's initial, following
/// the work like each pool that followed it until they are settled under the
/// next fold's maxima — see [`settle_risen`]. Panics when `entity` is not
/// rising.
pub fn mark_as_risen(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .take::<RisingComponent>()
        .expect("mark_as_risen is given a rising entity");
    for (declared, holding) in rise_pools(world, entity) {
        match holding {
            RisePool::GainedOnRisen => {
                pools::seed_following_work(world, entity, declared.id(), declared.initial());
            }
            RisePool::FollowsWork(_) | RisePool::Initial => {}
        }
    }
}

/// Settles the pools of `entity`, risen since the last fold, under the maxima
/// that fold left: moves each pool that followed the work or was gained on
/// risen from the line it last followed to where it now starts — the whole
/// maximum for a pool that followed the work, its initial for one gained on
/// risen — keeping what it gained or lost besides, and has it follow its
/// maximum from then on. Panics when such a pool does not follow work.
pub fn settle_risen(world: &mut World, entity: Entity) {
    for (declared, holding) in rise_pools(world, entity) {
        let pool = declared.id();
        let target = match holding {
            RisePool::FollowsWork(_) => PoolInitial::Full,
            RisePool::GainedOnRisen => declared.initial(),
            RisePool::Initial => continue,
        };
        let held = world
            .get::<PoolsComponent>(entity)
            .expect("a simulation entity carries a pool store");
        let last = match held.follows(pool) {
            Some(Follows::Work { line }) => line,
            Some(Follows::Maximum) | None => {
                panic!(
                    "a just-risen entity's pool that followed the work or was gained on risen follows work: {pool:?}"
                )
            }
        };
        let current = held.current(pool).expect("a pool following work is held");
        world.resource_scope(|world, registry: Mut<ContentRegistry>| {
            let maximum =
                entity_def::effective_stat(world, entity, registry.pool_def(pool).maximum_stat())
                    .expect("a risen entity carries the maximum of each pool following its work");
            step_to_line(
                world,
                &registry,
                entity,
                pool,
                (last, current),
                target.under(maximum),
                maximum,
            );
        });
        pools::follow_maximum(world, entity, pool);
    }
}

/// Each pool `entity`'s type declares, with how a rising entity of the type
/// holds it. Panics when the type never rises.
fn rise_pools(world: &World, entity: Entity) -> Vec<(Pool, RisePool)> {
    let def = world
        .resource::<ContentRegistry>()
        .def(entity_def::type_id(world, entity));
    let rise = def
        .rise
        .as_ref()
        .expect("a rising entity is of a type that rises");
    def.base_stats
        .pools()
        .map(|declared| {
            let holding = rise
                .pool(declared.id())
                .expect("a type that rises says how it holds each pool it declares");
            (*declared, holding)
        })
        .collect()
}

/// Moves `entity`'s `pool`, which follows its rise's work, by how far its line
/// moved from `last` to `line`, from the `current` value it holds: held
/// between empty and `maximum`, and a live health pool never emptied by it.
fn step_to_line(
    world: &mut World,
    registry: &ContentRegistry,
    entity: Entity,
    pool: PoolId,
    (last, current): (FixedU64, FixedU64),
    line: FixedU64,
    maximum: FixedU64,
) {
    let value = if line >= last {
        current.saturating_add(line - last).min(maximum)
    } else {
        let lowered = current.saturating_sub(last - line).min(maximum);
        match pool {
            PoolId::HEALTH if current > FixedU64::ZERO && lowered == FixedU64::ZERO => {
                FixedU64::DELTA
            }
            _ => lowered,
        }
    };
    pools::step_with_work(world, registry, entity, pool, value, line);
}
