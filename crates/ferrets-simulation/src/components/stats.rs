//! The unified per-entity stat store.
//!
//! Every modifiable numeric stat (health, damage, speed, …) lives here as a
//! [`FixedU64`] base value plus an effective value after unconditional
//! modifiers. Capability and structural data stay on their typed components;
//! only numeric, modifiable stats are stored here.
//!
//! Stats are content vocabulary: the engine pre-registers the built-ins
//! (whose ids are the [`StatId`] constants), and content may declare more via
//! `register_stat`. Values are fractional and never rounded in the simulation —
//! rounding is a display concern.

use bevy_ecs::prelude::*;
use ferrets_math::{FixedI64, FixedU64};

/// A handle to a registered stat, assigned in registration order.
///
/// The built-in stats occupy the low ids given by the associated constants;
/// content-declared stats follow in registration order. Identical content
/// registered in the same order resolves to identical ids on every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatId(u16);

impl StatId {
    /// Maximum health points. Current health is runtime state on the health
    /// component, not a stat.
    pub const MAX_HEALTH: StatId = StatId(0);
    /// Health points removed from a target by one hit, before armor.
    pub const DAMAGE: StatId = StatId(1);
    /// Flat damage subtracted from each incoming hit.
    pub const ARMOR: StatId = StatId(2);
    /// Movement speed in grid units per tick. Fractional, and authored below `1`
    /// for most entities, so it carries no floor: zero is a meaningful value that
    /// immobilises the entity, and a walk simply holds without advancing until the
    /// debuff lifts.
    pub const SPEED: StatId = StatId(3);
    /// Map-reveal radius in cells.
    pub const SIGHT_RANGE: StatId = StatId(4);
    /// Attack range in cells.
    pub const ATTACK_RANGE: StatId = StatId(5);
    /// Distance in cells at which enemies are engaged on the entity's own initiative.
    pub const ACQUIRE_RANGE: StatId = StatId(6);
    /// Ticks in one full attack cycle — the rate of fire (`DPS = damage / attack_period`).
    pub const ATTACK_PERIOD: StatId = StatId(7);
    /// Ticks into the attack cycle at which the hit lands (at most `attack_period`).
    pub const DAMAGE_POINT: StatId = StatId(8);
    /// Maximum energy available to spend on skills.
    pub const MAX_ENERGY: StatId = StatId(9);
    /// Energy regenerated per tick, toward [`MAX_ENERGY`](Self::MAX_ENERGY).
    pub const ENERGY_REGEN: StatId = StatId(10);

    /// Creates a stat id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more stats registered than StatId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Everything the engine knows about one built-in stat.
pub(crate) struct BuiltinStat {
    /// The handle content resolves this stat's name to.
    pub(crate) id: StatId,
    /// The name content declares the stat under.
    pub(crate) name: &'static str,
    /// Smallest effective value the engine tolerates. [`FixedU64::ZERO`] means the
    /// stat is meaningful at zero and is left to the general non-negative clamp.
    pub(crate) floor: FixedU64,
}

/// The built-in stats, registered first and in this order, so their assigned ids
/// equal the [`StatId`] constants above.
///
/// A non-zero floor marks a stat the engine reads as a whole number — a counter it
/// compares a phase against, or a distance in cells — where zero is a value the
/// consumer can never satisfy rather than simply meaning "none". Modifiers are
/// signed, so without the floor a debuff could reach values registration would have
/// rejected. Fractional stats take no floor: authored values below 1 would be
/// raised by one rather than guarded.
pub(crate) const BUILTIN_STATS: [BuiltinStat; 11] = [
    builtin(StatId::MAX_HEALTH, "max_health", FixedU64::ZERO),
    builtin(StatId::DAMAGE, "damage", FixedU64::ZERO),
    builtin(StatId::ARMOR, "armor", FixedU64::ZERO),
    // No floor: speed is fractional grid units per tick, and authored values sit
    // below 1, so any whole-number floor would raise them instead of guarding them.
    builtin(StatId::SPEED, "speed", FixedU64::ZERO),
    builtin(StatId::SIGHT_RANGE, "sight_range", FixedU64::ZERO),
    // Zero range can only be satisfied by standing inside the target's footprint.
    builtin(StatId::ATTACK_RANGE, "attack_range", FixedU64::ONE),
    builtin(StatId::ACQUIRE_RANGE, "acquire_range", FixedU64::ZERO),
    // The attack cycle counts 1..=period and the hit lands on the damage point, so
    // a zero for either is a phase the counter never reaches.
    builtin(StatId::ATTACK_PERIOD, "attack_period", FixedU64::ONE),
    builtin(StatId::DAMAGE_POINT, "damage_point", FixedU64::ONE),
    builtin(StatId::MAX_ENERGY, "max_energy", FixedU64::ZERO),
    builtin(StatId::ENERGY_REGEN, "energy_regen", FixedU64::ZERO),
];

/// Shorthand for one [`BUILTIN_STATS`] entry.
const fn builtin(id: StatId, name: &'static str, floor: FixedU64) -> BuiltinStat {
    BuiltinStat { id, name, floor }
}

// Floors and names are looked up by `StatId::index`, so every entry must sit at
// the slot its own id names.
const _: () = {
    let mut index = 0;
    while index < BUILTIN_STATS.len() {
        assert!(BUILTIN_STATS[index].id.index() == index);
        index += 1;
    }
};

/// The smallest effective value `stat` may fold to. Content-declared stats carry
/// no engine meaning, so they have no floor beyond the non-negative clamp.
fn floor_of(stat: StatId) -> FixedU64 {
    match BUILTIN_STATS.get(stat.index()) {
        Some(builtin) => builtin.floor,
        None => FixedU64::ZERO,
    }
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

/// How a [`Modifier`] folds into a stat's effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierOp {
    /// A signed delta added to the base, before the percentage layer.
    FlatAdd,
    /// A signed fraction summed with the other percentage modifiers and applied
    /// once — `0.5` is `+50%`, `-0.4` is `-40%`. Summing (not chaining) keeps the
    /// result order-independent.
    PercentAdd,
}

/// A single unconditional modifier over one stat. The magnitude is signed, so a
/// debuff is simply a negative modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    /// The stat this modifier applies to.
    pub stat: StatId,
    /// How the magnitude folds in.
    pub op: ModifierOp,
    /// The signed amount, interpreted per the op.
    pub magnitude: FixedI64,
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
