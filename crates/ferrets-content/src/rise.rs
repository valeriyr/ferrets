//! How an entity type rises into the world: the state it is in from the moment
//! it appears until it stands finished.

use std::collections::BTreeMap;

use ferrets_math::FixedU64;

use crate::{pool::PoolInitial, pool_def::PoolId};

/// Where a pool that follows its rising entity's work starts, below its
/// maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RisePoolInitial {
    /// This share of the maximum, from 0 to below 1.
    Share(FixedU64),
    /// This amount, below the maximum.
    Amount(FixedU64),
}

impl From<RisePoolInitial> for PoolInitial {
    fn from(initial: RisePoolInitial) -> Self {
        match initial {
            RisePoolInitial::Share(share) => PoolInitial::Share(share),
            RisePoolInitial::Amount(amount) => PoolInitial::Amount(amount),
        }
    }
}

/// How a rising entity holds a pool while it rises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RisePool {
    /// From this initial up to the maximum, following the work.
    FollowsWork(RisePoolInitial),
    /// From the pool's own initial, as on any entity.
    Initial,
    /// Not at all: the entity gains it at its initial once it has risen.
    GainedOnRisen,
}

/// How an entity type rises into the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiseDef {
    /// Ticks of work rising takes.
    time: u32,
    /// How a rising entity holds each pool the type declares.
    pools: BTreeMap<PoolId, RisePool>,
}

impl RiseDef {
    /// Creates a new `RiseDef` with the given data.
    ///
    /// Panics if `time` is `0` or `pools` names a pool twice.
    pub fn new(time: u32, pools: impl IntoIterator<Item = (PoolId, RisePool)>) -> Self {
        assert!(time > 0, "rise time must be greater than 0");
        let mut held = BTreeMap::new();
        for (pool, holding) in pools {
            assert!(
                held.insert(pool, holding).is_none(),
                "a rise names each pool once: {pool:?}"
            );
        }
        Self { time, pools: held }
    }

    /// Ticks of work rising takes.
    #[inline]
    pub fn time(&self) -> u32 {
        self.time
    }

    /// How a rising entity holds each pool the type declares.
    #[inline]
    pub fn pools(&self) -> impl Iterator<Item = (PoolId, RisePool)> + '_ {
        self.pools.iter().map(|(pool, holding)| (*pool, *holding))
    }

    /// How a rising entity holds `pool`, if this names it.
    pub fn pool(&self, pool: PoolId) -> Option<RisePool> {
        self.pools.get(&pool).copied()
    }
}
