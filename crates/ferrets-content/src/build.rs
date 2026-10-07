//! Content-defined construction.

use std::collections::BTreeMap;

use ferrets_math::FixedU64;

use crate::{
    pool::PoolInitial,
    pool_def::PoolId,
    work::{CrewLimit, Crewing, WorkPresence},
};

/// How a builder relates to a site it raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderAttendance {
    /// Joins the site's crew: its build order stays on the site and advances
    /// the progress one tick per tick, standing as the presence says.
    Crew(WorkPresence),
    /// Leaves the site unattended: its build order ends once the site is
    /// placed and paid for, and the site advances itself.
    Unattended,
    /// Works the site alone, hidden inside it, its build order advancing the
    /// progress; its supply is not counted while it is inside. It is consumed
    /// when the site completes instead of stepping back out, and a build order
    /// that ends early brings it back onto the map.
    Consumed,
}

impl BuilderAttendance {
    /// What decides how many builders of this attendance may raise one site at
    /// once.
    pub fn crewing(&self) -> Crewing {
        match self {
            BuilderAttendance::Crew(presence) => presence.crewing(),
            BuilderAttendance::Unattended | BuilderAttendance::Consumed => {
                Crewing::Counted(CrewLimit::ONE)
            }
        }
    }

    /// How the builder stands on a site it raises, or `None` when it does not
    /// stand there at all.
    pub fn presence(&self) -> Option<WorkPresence> {
        match self {
            BuilderAttendance::Crew(presence) => Some(presence.clone()),
            BuilderAttendance::Consumed => Some(WorkPresence::Hidden {
                crew: CrewLimit::ONE,
            }),
            BuilderAttendance::Unattended => None,
        }
    }
}

/// Content-defined construction catalogue: which entity types this entity can
/// build, and how it attends the sites it raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderDef {
    /// The entity types this entity can construct.
    builds: Vec<String>,
    /// How the builder relates to a site it raises.
    attendance: BuilderAttendance,
}

impl BuilderDef {
    /// Creates a new `BuilderDef` with the given data.
    ///
    /// Panics if `builds` is empty or contains an empty type name.
    pub fn new(
        builds: impl IntoIterator<Item = impl Into<String>>,
        attendance: BuilderAttendance,
    ) -> Self {
        let builds: Vec<String> = builds.into_iter().map(Into::into).collect();

        assert!(!builds.is_empty(), "builds must not be empty");
        assert!(
            builds.iter().all(|name| !name.is_empty()),
            "constructed type names must not be empty"
        );

        Self { builds, attendance }
    }

    /// Returns `true` if buildings of `type_name` can be constructed by this entity.
    pub fn can_build(&self, type_name: &str) -> bool {
        self.builds.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be constructed.
    pub fn builds(&self) -> impl Iterator<Item = &str> {
        self.builds.iter().map(String::as_str)
    }

    /// How the builder relates to a site it raises.
    #[inline]
    pub fn attendance(&self) -> &BuilderAttendance {
        &self.attendance
    }
}

/// Where a pool a construction site raises starts, below its maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiseStart {
    /// This share of the maximum, from 0 to below 1.
    Share(FixedU64),
    /// This amount, below the maximum.
    Amount(FixedU64),
}

impl From<RiseStart> for PoolInitial {
    fn from(start: RiseStart) -> Self {
        match start {
            RiseStart::Share(share) => PoolInitial::Share(share),
            RiseStart::Amount(amount) => PoolInitial::Amount(amount),
        }
    }
}

/// How a construction site holds a pool while it is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitePool {
    /// From this start up to the maximum, in step with the work.
    Rising(RiseStart),
    /// From the pool's own initial, as on any entity.
    Initial,
    /// Not at all: the finished building gains it at its initial.
    Withheld,
}

/// How an entity type is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildDef {
    /// Ticks of work a site takes.
    time: u32,
    /// How a site holds each pool the type declares.
    pools: BTreeMap<PoolId, SitePool>,
}

impl BuildDef {
    /// Creates a new `BuildDef` with the given data.
    ///
    /// Panics if `time` is `0` or `pools` names a pool twice.
    pub fn new(time: u32, pools: impl IntoIterator<Item = (PoolId, SitePool)>) -> Self {
        assert!(time > 0, "build time must be greater than 0");
        let mut sites = BTreeMap::new();
        for (pool, site) in pools {
            assert!(
                sites.insert(pool, site).is_none(),
                "a build names each pool once: {pool:?}"
            );
        }
        Self { time, pools: sites }
    }

    /// Ticks of work a site takes.
    #[inline]
    pub fn time(&self) -> u32 {
        self.time
    }

    /// How a site holds each pool the type declares.
    #[inline]
    pub fn pools(&self) -> impl Iterator<Item = (PoolId, SitePool)> + '_ {
        self.pools.iter().map(|(pool, site)| (*pool, *site))
    }

    /// How a site holds `pool`, if this names it.
    pub fn site(&self, pool: PoolId) -> Option<SitePool> {
        self.pools.get(&pool).copied()
    }
}
