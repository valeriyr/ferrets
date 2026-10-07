//! In-flight construction state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;

use crate::{
    components::{
        chase::ChaseState,
        pools::{self, Follows, PoolsComponent},
    },
    entity_def,
    simulation_id::SimulationId,
};
use ferrets_content::{
    build::SitePool,
    entity_type_def::EntityTypeId,
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    registry::ContentRegistry,
};

/// How a construction site's progress is advanced: by the build orders of the
/// builders on it, by the site itself once a builder that left it unattended
/// placed it, or not at all while it waits for a builder to take it up again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteWork {
    /// Builders are on the site, hidden inside it, on it or beside it, and
    /// their build orders advance the progress, one tick per tick each. The
    /// site never advances on its own.
    Crew {
        /// The builders whose build orders are on the site right now.
        builders: BTreeSet<SimulationId>,
    },
    /// No build order is on the site; it advances itself one tick per tick,
    /// and takes no crew.
    Unattended {
        /// The builder the site advances on behalf of: the one that placed it,
        /// or the one that has since taken it up.
        founder: SimulationId,
    },
    /// No build order is on the site and it does not advance: its crew left
    /// it standing with the progress raised so far, and any builder that
    /// raises the type takes it up again.
    Halted,
}

/// Marks a building whose construction is still in progress.
///
/// The progress lives on the site rather than on any one builder, so several
/// builders advance the same work and what they have raised so far outlives the
/// one that started it.
#[derive(Component, Debug)]
pub struct UnderConstructionComponent {
    /// Ticks of work put into the site.
    pub progress: u32,
    /// How the progress is advanced, and by whom the site was placed.
    pub work: SiteWork,
}

impl UnderConstructionComponent {
    /// A site with no work put in yet, advanced as `work` says.
    pub fn new(work: SiteWork) -> Self {
        Self { progress: 0, work }
    }

    /// Steps the work for the `tender` offering to advance the site: a halted
    /// site is taken up as that tender's own, an unattended one whose tender
    /// has gone is halted again, and a site with a crew on it is being worked
    /// by that crew already.
    pub fn tend(&mut self, tender: Option<SimulationId>) {
        match (&self.work, tender) {
            (SiteWork::Halted, Some(founder)) => self.work = SiteWork::Unattended { founder },
            (SiteWork::Unattended { .. }, None) => self.work = SiteWork::Halted,
            (SiteWork::Crew { .. }, _)
            | (SiteWork::Halted, None)
            | (SiteWork::Unattended { .. }, Some(_)) => {}
        }
    }
}

/// Moves each pool of the site `entity` that follows its work by how far
/// the work's line moved since last read: the line stands at `start +
/// (maximum − start) × progress ÷ build time`, `start` being the rise
/// start under the maximum, both read as they stand now, so what the site
/// lost or gained besides stays as it was. Held between empty and the
/// maximum; a live health pool is never emptied by it. Panics when `entity`
/// is not a site with a pool following its work, or its work is past its
/// build time.
pub fn rise(world: &mut World, entity: Entity) {
    let progress = world
        .entity(entity)
        .get::<UnderConstructionComponent>()
        .expect("rise is given a site under construction")
        .progress;
    let store = world
        .get::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store");
    let following: Vec<(PoolId, PoolInitial, FixedU64, FixedU64)> = site_pools(world, entity)
        .into_iter()
        .filter_map(
            |(declared, site)| match (site, store.follows(declared.id())) {
                (SitePool::Rising(start), Some(Follows::Work { line })) => Some((
                    declared.id(),
                    PoolInitial::from(start),
                    line,
                    store
                        .current(declared.id())
                        .expect("a pool following work is held"),
                )),
                (SitePool::Rising(_), Some(Follows::Maximum) | None) => {
                    unreachable!("a site's rising pool follows its work: {:?}", declared.id())
                }
                (SitePool::Initial | SitePool::Withheld, _) => None,
            },
        )
        .collect();
    assert!(
        !following.is_empty(),
        "rise is given a site with a pool following its work"
    );
    let build_time = entity_def::of(world, entity)
        .build_time()
        .expect("a site with a pool following its work is of a constructible type");
    assert!(
        progress <= build_time,
        "a site's work stops at its build time: {progress} > {build_time}"
    );
    world.resource_scope(|world, registry: Mut<ContentRegistry>| {
        for (pool, initial, last, current) in following {
            let maximum =
                entity_def::effective_stat(world, entity, registry.pool_def(pool).maximum_stat())
                    .expect("a site carries the maximum of each pool following its work");
            let start = initial.under(maximum);
            // Widened so the product neither overflows nor rounds before the
            // division; at most `maximum − start`, it narrows back exactly.
            let gained = u128::from((maximum - start).to_bits()) * u128::from(progress)
                / u128::from(build_time);
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

/// Marks the site `entity` as built: takes its construction state off and
/// gives it each pool its type withholds on a site at that pool's initial,
/// following the work like each pool the site raised until they are settled
/// under the next fold's maxima — see [`settle_built`]. Panics when `entity`
/// is not a site.
pub fn mark_as_built(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .take::<UnderConstructionComponent>()
        .expect("mark_as_built is given a site under construction");
    for (declared, site) in site_pools(world, entity) {
        match site {
            SitePool::Withheld => {
                pools::seed_rising(world, entity, declared.id(), declared.initial());
            }
            SitePool::Rising(_) | SitePool::Initial => {}
        }
    }
}

/// Settles the pools of the building `entity`, built since the last fold,
/// under the maxima that fold left: moves each pool its site raised or
/// withheld from the line it last followed to where it now starts — the
/// whole maximum for a raised pool, its initial for a withheld one — keeping
/// what it gained or lost besides, and has it follow its maximum from then
/// on. Panics when a pool its build raises or withholds does not follow work.
pub fn settle_built(world: &mut World, entity: Entity) {
    for (declared, site) in site_pools(world, entity) {
        let pool = declared.id();
        let target = match site {
            SitePool::Rising(_) => PoolInitial::Full,
            SitePool::Withheld => declared.initial(),
            SitePool::Initial => continue,
        };
        let held = world
            .get::<PoolsComponent>(entity)
            .expect("a simulation entity carries a pool store");
        let last = match held.follows(pool) {
            Some(Follows::Work { line }) => line,
            Some(Follows::Maximum) | None => {
                panic!("a just-built building's raised or withheld pool follows work: {pool:?}")
            }
        };
        let current = held.current(pool).expect("a pool following work is held");
        world.resource_scope(|world, registry: Mut<ContentRegistry>| {
            let maximum =
                entity_def::effective_stat(world, entity, registry.pool_def(pool).maximum_stat())
                    .expect("a building carries the maximum of each pool following its work");
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

/// Per-entity in-flight construction state.
#[derive(Component, Debug, Default)]
pub struct BuildComponent {
    /// The building being constructed, once it has been placed on the map or an
    /// already-started site has been taken up.
    pub building: Option<SimulationId>,
    /// The last chase round toward the site; identical rounds accumulate
    /// until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}

/// Marks an entity raised over a resource source it took the remaining
/// amount of, which comes back as that source when the entity dies.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverbuiltComponent {
    /// The source type put back with what is left.
    pub uncovers: EntityTypeId,
    /// The cell the covered source's footprint was anchored at.
    pub anchor: CellPos,
}

/// Each pool `entity`'s type declares, with how a site of the type holds
/// it. Panics when the type cannot be built.
fn site_pools(world: &World, entity: Entity) -> Vec<(Pool, SitePool)> {
    let def = world
        .resource::<ContentRegistry>()
        .def(entity_def::type_id(world, entity));
    let build = def
        .build
        .as_ref()
        .expect("a site is of a constructible type");
    def.base_stats
        .pools()
        .map(|declared| {
            let site = build
                .site(declared.id())
                .expect("a constructible type says how a site holds each pool it declares");
            (*declared, site)
        })
        .collect()
}

/// Moves `entity`'s `pool`, which follows a site's work, by how far its line
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
