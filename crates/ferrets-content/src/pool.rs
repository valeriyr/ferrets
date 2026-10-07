//! A pool as an entity type declares it.

use ferrets_math::FixedU64;
use serde::{Deserialize, Serialize};

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

/// The value a pool starts at, against the maximum it stands under then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolInitial {
    /// The whole maximum.
    Full,
    /// This share of the maximum, from 0 to 1.
    Share(FixedU64),
    /// This amount, held under the maximum.
    Amount(FixedU64),
}

impl PoolInitial {
    /// The value this initial comes to under `maximum`.
    pub fn under(self, maximum: FixedU64) -> FixedU64 {
        match self {
            PoolInitial::Full => maximum,
            PoolInitial::Share(share) => maximum.saturating_mul(share).min(maximum),
            PoolInitial::Amount(amount) => amount.min(maximum),
        }
    }
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
    /// Where the pool starts.
    initial: PoolInitial,
}

impl Pool {
    /// Creates a new `Pool` of `id`, whose stats are the ones `def` names.
    pub const fn new(
        id: PoolId,
        def: PoolDef,
        maximum: FixedU64,
        regen: FixedU64,
        drain: FixedU64,
        initial: PoolInitial,
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
            initial,
        }
    }

    /// A built-in pool of `id`, whose stats are the ones its built-in
    /// definition names. Panics when `id` is no built-in pool.
    pub fn builtin(
        id: PoolId,
        maximum: FixedU64,
        regen: FixedU64,
        drain: FixedU64,
        initial: PoolInitial,
    ) -> Self {
        let builtin = POOL_BUILTINS
            .get(id.index())
            .unwrap_or_else(|| panic!("{id:?} is a built-in pool"));
        Self::new(id, builtin.def, maximum, regen, drain, initial)
    }

    /// A health pool of `maximum`, neither regenerating nor draining,
    /// starting full.
    pub fn health(maximum: u32) -> Self {
        Self::builtin(
            PoolId::HEALTH,
            FixedU64::from_num(maximum),
            FixedU64::ZERO,
            FixedU64::ZERO,
            PoolInitial::Full,
        )
    }

    /// An energy pool of `maximum`, neither regenerating nor draining,
    /// starting full.
    pub fn energy(maximum: u32) -> Self {
        Self::builtin(
            PoolId::ENERGY,
            FixedU64::from_num(maximum),
            FixedU64::ZERO,
            FixedU64::ZERO,
            PoolInitial::Full,
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

    /// Where the pool starts.
    #[inline]
    pub fn initial(self) -> PoolInitial {
        self.initial
    }

    /// Checks that `initial` can start the pool: a share no more than 1, an
    /// amount no more than the base maximum, and health above empty. Panics
    /// otherwise, the message opening with `opening`, what starts the pool.
    pub fn validate_initial(self, initial: PoolInitial, opening: &str) {
        let start = match initial {
            PoolInitial::Full => self.maximum(),
            PoolInitial::Share(share) => {
                assert!(
                    share <= FixedU64::ONE,
                    "{opening} at a share of {share}, above 1"
                );
                share
            }
            PoolInitial::Amount(amount) => {
                assert!(
                    amount <= self.maximum(),
                    "{opening} at {amount}, above its maximum of {}",
                    self.maximum()
                );
                amount
            }
        };
        assert!(
            self.id != PoolId::HEALTH || start > FixedU64::ZERO,
            "{opening} at 0"
        );
    }

    /// The stats the pool fills, with their base values: its maximum, its
    /// regeneration and its drain.
    #[inline]
    pub fn stats(self) -> [PoolStat; 3] {
        [self.maximum, self.regen, self.drain]
    }
}
