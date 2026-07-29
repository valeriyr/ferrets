//! The unified per-entity stat store.
//!
//! Every modifiable numeric stat (health, damage, speed, …) lives here as a
//! [`FixedU64`] base value plus an effective value after unconditional modifiers.
//! Capability and structural data stay on their typed components; only numeric,
//! modifiable stats are stored here.

use bevy_ecs::prelude::*;
use ferrets_math::{FixedI64, FixedU64};

use crate::content::{
    stats::floor_of,
    stats::{Modifier, ModifierOp, StatId},
};

/// One stat's base value and its effective value after unconditional modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatCell {
    /// The authored value, before any modifier folds in.
    base: FixedU64,
    /// The value after unconditional modifiers; equals `base` until the modifier
    /// pipeline recomputes it.
    effective: FixedU64,
}

impl StatCell {
    #[inline]
    fn new(base: FixedU64) -> Self {
        Self {
            base,
            effective: base,
        }
    }
}

/// Per-entity store of stat base and effective values, indexed by [`StatId`].
///
/// Only the stats an entity's type declares are present; reading an undeclared
/// stat returns `None`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsComponent {
    /// Indexed by [`StatId::index`]; `None` for stats this entity does not have.
    cells: Vec<Option<StatCell>>,
}

impl StatsComponent {
    /// Sets a stat's base value, resetting its effective value to match, and
    /// growing the store as needed. Used at spawn and when the base changes.
    pub fn set_base(&mut self, stat: StatId, base: FixedU64) {
        let index = stat.index();
        if index >= self.cells.len() {
            self.cells.resize(index + 1, None);
        }
        self.cells[index] = Some(StatCell::new(base));
    }

    /// The base value of `stat`, or `None` if the entity does not have it.
    #[inline]
    pub fn base(&self, stat: StatId) -> Option<FixedU64> {
        self.cell(stat).map(|c| c.base)
    }

    /// The base value of `stat` truncated to a whole number, or `None` if the
    /// entity does not have it — for integer-consuming callers.
    #[inline]
    pub fn base_as_u32(&self, stat: StatId) -> Option<u32> {
        self.base(stat).map(|value| value.to_num::<u32>())
    }

    /// The effective value of `stat` after unconditional modifiers, or `None`
    /// if the entity does not have it.
    #[inline]
    pub fn effective(&self, stat: StatId) -> Option<FixedU64> {
        self.cell(stat).map(|c| c.effective)
    }

    /// The effective value of `stat` truncated to a whole number, or `None` if
    /// the entity does not have it — for integer-consuming callers.
    #[inline]
    pub fn effective_as_u32(&self, stat: StatId) -> Option<u32> {
        self.effective(stat).map(|value| value.to_num::<u32>())
    }

    /// `true` if the entity has `stat` declared.
    #[inline]
    pub fn has(&self, stat: StatId) -> bool {
        self.cell(stat).is_some()
    }

    /// The present cell for `stat`, if any.
    #[inline]
    fn cell(&self, stat: StatId) -> Option<StatCell> {
        self.cells.get(stat.index()).copied().flatten()
    }

    /// Recomputes every present stat's effective value from its base and the
    /// modifiers targeting it, holding the result at the stat's floor. Modifiers
    /// for stats the entity does not have are ignored. The fold is
    /// order-independent.
    pub fn recompute(&mut self, modifiers: &[Modifier]) {
        for (index, cell) in self.cells.iter_mut().enumerate() {
            let Some(cell) = cell else { continue };
            let stat = StatId::from_index(index);
            cell.effective =
                combine(cell.base, modifiers.iter().filter(|m| m.stat == stat)).max(floor_of(stat));
        }
    }
}

/// Folds `base` and the modifiers targeting one stat into an effective value:
/// `(base + Σ flat) × (1 + Σ percent)`, clamped at zero. Both sums are
/// order-independent, so the modifier order never changes the result.
fn combine<'a>(base: FixedU64, modifiers: impl Iterator<Item = &'a Modifier>) -> FixedU64 {
    let mut flat = FixedI64::ZERO;
    let mut percent = FixedI64::ZERO;
    for modifier in modifiers {
        match modifier.op {
            ModifierOp::FlatAdd => flat = flat.saturating_add(modifier.magnitude),
            ModifierOp::PercentAdd => percent = percent.saturating_add(modifier.magnitude),
        }
    }
    let scaled = FixedI64::from_num(base)
        .saturating_add(flat)
        .saturating_mul(FixedI64::ONE.saturating_add(percent));
    FixedU64::from_num(scaled.max(FixedI64::ZERO))
}
