//! The unified per-entity stat store.
//!
//! Every modifiable numeric stat (health, damage, speed, …) lives here as a
//! [`FixedU64`] base value plus an effective value after unconditional modifiers.
//! Capability and structural data stay on their typed components; only numeric,
//! modifiable stats are stored here.

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use crate::content::{
    entity_stats::{self, EntityStatId},
    stats::{EntityModifier, StatStore},
};

/// Per-entity store of stat base and effective values, indexed by
/// [`EntityStatId`].
///
/// Only the stats an entity's type declares are present; reading an undeclared
/// stat returns `None`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsComponent(StatStore);

impl StatsComponent {
    /// Sets a stat's base value, resetting its effective value to match, and
    /// growing the store as needed. Used at spawn and when the base changes.
    pub fn set_base(&mut self, stat: EntityStatId, base: FixedU64) {
        self.0.set_base(stat.index(), base);
    }

    /// The base value of `stat`, or `None` if the entity does not have it.
    #[inline]
    pub fn base(&self, stat: EntityStatId) -> Option<FixedU64> {
        self.0.base(stat.index())
    }

    /// The base value of `stat` truncated to a whole number, or `None` if the
    /// entity does not have it — for integer-consuming callers.
    #[inline]
    pub fn base_as_u32(&self, stat: EntityStatId) -> Option<u32> {
        self.base(stat).map(|value| value.to_num::<u32>())
    }

    /// The effective value of `stat` after unconditional modifiers, or `None`
    /// if the entity does not have it.
    #[inline]
    pub fn effective(&self, stat: EntityStatId) -> Option<FixedU64> {
        self.0.effective(stat.index())
    }

    /// The effective value of `stat` truncated to a whole number, or `None` if
    /// the entity does not have it — for integer-consuming callers.
    #[inline]
    pub fn effective_as_u32(&self, stat: EntityStatId) -> Option<u32> {
        self.effective(stat).map(|value| value.to_num::<u32>())
    }

    /// `true` if the entity has `stat` declared.
    #[inline]
    pub fn has(&self, stat: EntityStatId) -> bool {
        self.0.has(stat.index())
    }

    /// Recomputes every present stat's effective value from its base and the
    /// modifiers targeting it, holding the result at the stat's floor.
    pub fn recompute(&mut self, modifiers: &[EntityModifier]) {
        let targeting: Vec<_> = modifiers
            .iter()
            .map(|m| (m.stat.index(), m.op, m.magnitude))
            .collect();
        self.0.recompute(&targeting, entity_stats::floor_of);
    }
}
