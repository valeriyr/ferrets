//! A pool as an entity type declares it: which pool, and the stats it fills
//! with their base values.

use ferrets_math::FixedU64;

use crate::{
    entity_stats::EntityStatId,
    pool_def::{POOL_BUILTINS, PoolDef, PoolId},
};

/// One stat a declared pool fills, and its base value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStat {
    /// The stat filled.
    pub stat: EntityStatId,
    /// Its base value.
    pub value: FixedU64,
}

/// A pool an entity type declares, with the stats it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pool {
    /// The pool declared.
    id: PoolId,
    /// The pool's maximum.
    maximum: PoolStat,
    /// The pool's regeneration.
    regen: PoolStat,
    /// The pool's drain.
    drain: PoolStat,
}

impl Pool {
    /// Creates a new `Pool` of `id`, whose stats are the ones `def` names.
    pub const fn new(
        id: PoolId,
        def: PoolDef,
        maximum: FixedU64,
        regen: FixedU64,
        drain: FixedU64,
    ) -> Self {
        Self {
            id,
            maximum: PoolStat {
                stat: def.maximum_stat(),
                value: maximum,
            },
            regen: PoolStat {
                stat: def.regen_stat(),
                value: regen,
            },
            drain: PoolStat {
                stat: def.drain_stat(),
                value: drain,
            },
        }
    }

    /// A built-in pool of `id`, whose stats are the ones its built-in
    /// definition names. Panics when `id` is no built-in pool.
    pub fn builtin(id: PoolId, maximum: FixedU64, regen: FixedU64, drain: FixedU64) -> Self {
        let builtin = POOL_BUILTINS
            .get(id.index())
            .unwrap_or_else(|| panic!("{id:?} is a built-in pool"));
        Self::new(id, builtin.def, maximum, regen, drain)
    }

    /// A health pool of `maximum`, neither regenerating nor draining.
    pub fn health(maximum: u32) -> Self {
        Self::builtin(
            PoolId::HEALTH,
            FixedU64::from_num(maximum),
            FixedU64::ZERO,
            FixedU64::ZERO,
        )
    }

    /// An energy pool of `maximum`, neither regenerating nor draining.
    pub fn energy(maximum: u32) -> Self {
        Self::builtin(
            PoolId::ENERGY,
            FixedU64::from_num(maximum),
            FixedU64::ZERO,
            FixedU64::ZERO,
        )
    }

    /// The pool declared.
    #[inline]
    pub fn id(self) -> PoolId {
        self.id
    }

    /// The base value of the pool's maximum.
    #[inline]
    pub fn maximum(self) -> FixedU64 {
        self.maximum.value
    }

    /// The base value of the pool's regeneration.
    #[inline]
    pub fn regen(self) -> FixedU64 {
        self.regen.value
    }

    /// The base value of the pool's drain.
    #[inline]
    pub fn drain(self) -> FixedU64 {
        self.drain.value
    }

    /// The stats the pool fills, with their base values: its maximum, its
    /// regeneration and its drain.
    #[inline]
    pub fn stats(self) -> [PoolStat; 3] {
        [self.maximum, self.regen, self.drain]
    }
}
