//! The pool vocabulary: handles, and the relationship between the stats that
//! make a pool — a current value held under a maximum and moved each tick by
//! its rates.

use crate::entity_stats::EntityStatId;

/// Which of a pool's stats a stat is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolRole {
    /// The stat the current value is held under.
    Maximum,
    /// The stat the current value rises by each tick.
    Regen,
    /// The stat the current value falls by each tick.
    Drain,
}

/// A handle to a registered pool, assigned in registration order.
///
/// The built-in pools occupy the low ids given by the associated constants;
/// content-declared pools follow in registration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PoolId(u16);

impl PoolId {
    /// The pool an entity lives by.
    pub const HEALTH: PoolId = PoolId(0);
    /// The pool skills and upkeeps draw on.
    pub const ENERGY: PoolId = PoolId(1);

    /// The handle's position in registration order.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The handle at `index` in registration order.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("fewer pools than a u16 counts"))
    }
}

/// The stats a pool is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolDef {
    /// The stat the current value is held under.
    maximum_stat: EntityStatId,
    /// The stat the current value rises by each tick.
    regen_stat: EntityStatId,
    /// The stat the current value falls by each tick.
    drain_stat: EntityStatId,
}

impl PoolDef {
    /// Creates a new `PoolDef` with the given data.
    pub const fn new(
        maximum_stat: EntityStatId,
        regen_stat: EntityStatId,
        drain_stat: EntityStatId,
    ) -> Self {
        Self {
            maximum_stat,
            regen_stat,
            drain_stat,
        }
    }

    /// The stat the current value is held under.
    #[inline]
    pub const fn maximum_stat(self) -> EntityStatId {
        self.maximum_stat
    }

    /// The stat the current value rises by each tick.
    #[inline]
    pub const fn regen_stat(self) -> EntityStatId {
        self.regen_stat
    }

    /// The stat the current value falls by each tick.
    #[inline]
    pub const fn drain_stat(self) -> EntityStatId {
        self.drain_stat
    }

    /// The role `stat` plays in this pool, if it is one of its stats.
    pub fn role_of(self, stat: EntityStatId) -> Option<PoolRole> {
        if stat == self.maximum_stat {
            Some(PoolRole::Maximum)
        } else if stat == self.regen_stat {
            Some(PoolRole::Regen)
        } else if stat == self.drain_stat {
            Some(PoolRole::Drain)
        } else {
            None
        }
    }
}

/// Everything the engine knows about one built-in pool.
pub(crate) struct BuiltinPool {
    /// The handle content resolves this pool's name to.
    pub(crate) id: PoolId,
    /// The name content declares the pool under.
    pub(crate) name: &'static str,
    /// The stats the pool is made of.
    pub(crate) def: PoolDef,
}

/// The engine's built-in pools, each at the slot its id names.
pub(crate) const POOL_BUILTINS: [BuiltinPool; 2] = [
    BuiltinPool {
        id: PoolId::HEALTH,
        name: "health",
        def: PoolDef::new(
            EntityStatId::MAX_HEALTH,
            EntityStatId::HEALTH_REGEN,
            EntityStatId::HEALTH_DRAIN,
        ),
    },
    BuiltinPool {
        id: PoolId::ENERGY,
        name: "energy",
        def: PoolDef::new(
            EntityStatId::MAX_ENERGY,
            EntityStatId::ENERGY_REGEN,
            EntityStatId::ENERGY_DRAIN,
        ),
    },
];

// Definitions are looked up by `PoolId::index`, so every entry must sit at the
// slot its own id names.
const _: () = {
    let mut index = 0;
    while index < POOL_BUILTINS.len() {
        assert!(POOL_BUILTINS[index].id.index() == index);
        index += 1;
    }
};

// A stat belongs to one pool in one role, so no two of the built-in pools'
// stats are the same stat.
const _: () = {
    let mut first = 0;
    while first < POOL_BUILTINS.len() * 3 {
        let mut second = first + 1;
        while second < POOL_BUILTINS.len() * 3 {
            assert!(builtin_stat(first).index() != builtin_stat(second).index());
            second += 1;
        }
        first += 1;
    }
};

/// The `slot`-th stat of the built-in pools, three to a pool.
const fn builtin_stat(slot: usize) -> EntityStatId {
    let def = &POOL_BUILTINS[slot / 3].def;
    match slot % 3 {
        0 => def.maximum_stat,
        1 => def.regen_stat,
        2 => def.drain_stat,
        _ => panic!("a slot modulo three is below three"),
    }
}
