//! The stat machinery shared by every stat group: the modifier vocabulary, how
//! magnitudes fold into effective values, the built-in-stat shape whose floors
//! bound the folds, and the store the folds run in.
//!
//! Values are fractional and never rounded in the simulation — rounding is a
//! display concern. Nothing here knows a stat group: the typed ids are gates
//! carried through as opaque indices.

use ferrets_math::{FixedI64, FixedU64};

use crate::content::{entity_stats::EntityStatId, player_stats::PlayerStatId};

/// How a modifier's magnitude folds into a stat's effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierOp {
    /// A signed delta added to the base, before the percentage layer.
    FlatAdd,
    /// A signed fraction summed with the other percentage modifiers and applied
    /// once — `0.5` is `+50%`, `-0.4` is `-40%`. Summing (not chaining) keeps the
    /// result order-independent.
    PercentAdd,
}

/// A single unconditional modifier over one stat of the `StatId` group. The
/// magnitude is signed, so a debuff is simply a negative modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifier<StatId> {
    /// The stat this modifier applies to.
    pub stat: StatId,
    /// How the magnitude folds in.
    pub op: ModifierOp,
    /// The signed amount, interpreted per the op.
    pub magnitude: FixedI64,
}

/// A modifier over one entity stat.
pub type EntityModifier = Modifier<EntityStatId>;

/// A modifier over one player stat.
pub type PlayerModifier = Modifier<PlayerStatId>;

/// Folds `base` and modifier `(op, magnitude)` pairs into an effective value:
/// `(base + Σ flat) × (1 + Σ percent)`, clamped at zero. Both sums are
/// order-independent, so the modifier order never changes the result.
///
/// The one fold every stat store uses, whatever its stats are properties of, so
/// modifier arithmetic cannot drift between them.
pub(crate) fn fold(
    base: FixedU64,
    modifiers: impl Iterator<Item = (ModifierOp, FixedI64)>,
) -> FixedU64 {
    let mut flat = FixedI64::ZERO;
    let mut percent = FixedI64::ZERO;
    for (op, magnitude) in modifiers {
        match op {
            ModifierOp::FlatAdd => flat = flat.saturating_add(magnitude),
            ModifierOp::PercentAdd => percent = percent.saturating_add(magnitude),
        }
    }
    let scaled = FixedI64::from_num(base)
        .saturating_add(flat)
        .saturating_mul(FixedI64::ONE.saturating_add(percent));
    FixedU64::from_num(scaled.max(FixedI64::ZERO))
}

/// Everything the engine knows about one built-in stat of the `K` group.
pub(crate) struct BuiltinStat<K> {
    /// The handle content resolves this stat's name to.
    pub(crate) id: K,
    /// The name content declares the stat under.
    pub(crate) name: &'static str,
    /// Smallest effective value the engine tolerates. [`FixedU64::ZERO`] means the
    /// stat is meaningful at zero and is left to the general non-negative clamp.
    ///
    /// A non-zero floor marks a stat where zero is a value the consumer can never
    /// satisfy rather than simply meaning "none" — a counter, a distance in cells,
    /// or a pool ceiling. Modifiers are signed, so without the floor a debuff
    /// could reach values registration would have rejected. Fractional stats take
    /// no floor: authored values below 1 would be raised by one rather than
    /// guarded.
    pub(crate) floor: FixedU64,
}

/// Shorthand for one built-in stat entry of any group.
pub(crate) const fn builtin<K>(id: K, name: &'static str, floor: FixedU64) -> BuiltinStat<K> {
    BuiltinStat { id, name, floor }
}

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

/// Index-transparent store of stat cells — the one implementation every stat
/// group computes on.
///
/// It knows registration indices, values, and the floors handed to it, never the
/// typed ids: the wrappers around it are the gates that keep the groups apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StatStore {
    /// Indexed by registration index; `None` for stats the owner does not have.
    cells: Vec<Option<StatCell>>,
}

impl StatStore {
    /// Sets a stat's base value, resetting its effective value to match, and
    /// growing the store as needed.
    pub(crate) fn set_base(&mut self, index: usize, base: FixedU64) {
        if index >= self.cells.len() {
            self.cells.resize(index + 1, None);
        }
        self.cells[index] = Some(StatCell::new(base));
    }

    /// The base value at `index`, or `None` if the owner does not have it.
    pub(crate) fn base(&self, index: usize) -> Option<FixedU64> {
        self.cell(index).map(|c| c.base)
    }

    /// The effective value at `index`, or `None` if the owner does not have it.
    pub(crate) fn effective(&self, index: usize) -> Option<FixedU64> {
        self.cell(index).map(|c| c.effective)
    }

    /// `true` if the owner has the stat at `index`.
    pub(crate) fn has(&self, index: usize) -> bool {
        self.cell(index).is_some()
    }

    /// The present cell at `index`, if any.
    #[inline]
    fn cell(&self, index: usize) -> Option<StatCell> {
        self.cells.get(index).copied().flatten()
    }

    /// Recomputes every present stat's effective value from its base and the
    /// `(index, op, magnitude)` entries targeting it, holding each result at the
    /// floor `floor_of` names for its index. Entries for stats the owner does
    /// not have are ignored. The fold is order-independent.
    ///
    /// Callers hand in their own group's arm already reduced to indices — the
    /// store never sees the other arm, or any typed id at all.
    pub(crate) fn recompute(
        &mut self,
        modifiers: &[(usize, ModifierOp, FixedI64)],
        floor_of: impl Fn(usize) -> FixedU64,
    ) {
        for (index, cell) in self.cells.iter_mut().enumerate() {
            let Some(cell) = cell else { continue };
            let targeting = modifiers
                .iter()
                .filter(|(target, _, _)| *target == index)
                .map(|&(_, op, magnitude)| (op, magnitude));
            cell.effective = fold(cell.base, targeting).max(floor_of(index));
        }
    }
}
