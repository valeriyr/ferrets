//! The stat machinery shared by every stat group: the modifier vocabulary, how
//! magnitudes fold into effective values, and the built-in-stat shape whose
//! floors bound the folds.
//!
//! Values are fractional and never rounded in the simulation — rounding is a
//! display concern. Nothing here knows a stat group: the typed ids are gates
//! carried through as opaque indices.

use ferrets_math::{FixedI64, FixedU64};

use crate::{entity_stats::EntityStatId, player_stats::PlayerStatId};

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
pub fn fold(base: FixedU64, modifiers: impl Iterator<Item = (ModifierOp, FixedI64)>) -> FixedU64 {
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
