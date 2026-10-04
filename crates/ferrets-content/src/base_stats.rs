//! The base stats an entity type gives its instances: those written on their
//! own, and those the pools it declares fill.

use std::collections::BTreeMap;

use ferrets_math::FixedU64;

use crate::{entity_stats::EntityStatId, pool::Pool, pool_def::PoolId};

/// A type's base stats: those written on their own, and those its pools fill.
/// A stat is one or the other, never both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseStats {
    /// The stats written on their own, outside any pool.
    own: BTreeMap<EntityStatId, FixedU64>,
    /// The pools declared, each with the stats it fills.
    pools: BTreeMap<PoolId, Pool>,
}

impl BaseStats {
    /// Creates an empty `BaseStats`.
    pub(crate) fn new() -> Self {
        Self {
            own: BTreeMap::new(),
            pools: BTreeMap::new(),
        }
    }

    /// The base value of `stat`, whether written on its own or filled by a
    /// declared pool.
    pub fn get(&self, stat: EntityStatId) -> Option<FixedU64> {
        self.own.get(&stat).copied().or_else(|| {
            self.pools
                .values()
                .flat_map(|pool| pool.stats())
                .find(|filled| filled.stat == stat)
                .map(|filled| filled.value)
        })
    }

    /// Every base stat with its value: those written on their own, then each
    /// declared pool's.
    pub fn iter(&self) -> impl Iterator<Item = (EntityStatId, FixedU64)> + '_ {
        self.own.iter().map(|(&stat, &value)| (stat, value)).chain(
            self.pools
                .values()
                .flat_map(|pool| pool.stats())
                .map(|filled| (filled.stat, filled.value)),
        )
    }

    /// The stats written on their own, with their values.
    pub fn own(&self) -> impl Iterator<Item = (EntityStatId, FixedU64)> + '_ {
        self.own.iter().map(|(&stat, &value)| (stat, value))
    }

    /// Sets `stat` on its own. Panics when a declared pool fills it.
    pub fn insert_own(&mut self, stat: EntityStatId, value: FixedU64) {
        assert!(
            self.filling_pool(stat).is_none(),
            "{stat:?} is filled by a declared pool; declare the pool's values instead"
        );
        self.own.insert(stat, value);
    }

    /// Declares `pool`, replacing an earlier declaration of the same pool.
    /// Panics when one of its stats is already set on its own.
    pub fn insert_pool(&mut self, pool: Pool) {
        for filled in pool.stats() {
            assert!(
                !self.own.contains_key(&filled.stat),
                "{:?} is set on its own and filled by the {:?} pool; declare it once",
                filled.stat,
                pool.id()
            );
        }
        self.pools.insert(pool.id(), pool);
    }

    /// The declared pool `id`, if any.
    pub fn pool(&self, id: PoolId) -> Option<&Pool> {
        self.pools.get(&id)
    }

    /// Every declared pool, in pool order.
    pub fn pools(&self) -> impl Iterator<Item = &Pool> + '_ {
        self.pools.values()
    }

    /// The declared pool that fills `stat`, if any.
    fn filling_pool(&self, stat: EntityStatId) -> Option<&Pool> {
        self.pools
            .values()
            .find(|pool| pool.stats().iter().any(|filled| filled.stat == stat))
    }
}
