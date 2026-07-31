//! The stat vocabulary: handles, the engine's built-in stats, and the modifier
//! value content authors over them.
//!
//! Values are fractional and never rounded in the simulation — rounding is a
//! display concern.

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
    /// Health regenerated per tick, toward [`MAX_HEALTH`](Self::MAX_HEALTH).
    pub const HEALTH_REGEN: StatId = StatId(11);
    /// How fast one worker mends, as a multiple of the target's production rate:
    /// `1` restores the target in the time it takes to produce one.
    pub const REPAIR_SPEED: StatId = StatId(12);
    /// Share of a target's own cost charged for mending it in full — `0.25` bills a
    /// quarter of the price to restore an empty pool.
    pub const REPAIR_COST_FACTOR: StatId = StatId(13);
    /// How close a mender must be to its work, in cells.
    pub const REPAIR_RANGE: StatId = StatId(14);
    /// How close a builder must be to a site it is raising, in cells.
    pub const BUILD_RANGE: StatId = StatId(15);
    /// How close a carrier must be to a source it works, in cells.
    pub const HARVEST_RANGE: StatId = StatId(16);

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
/// A non-zero floor marks a stat where zero is a value the consumer can never
/// satisfy rather than simply meaning "none" — a counter, a distance in cells, or a
/// pool ceiling. Modifiers are signed, so without the floor a debuff could reach
/// values registration would have rejected. Fractional stats take no floor:
/// authored values below 1 would be raised by one rather than guarded.
pub(crate) const BUILTIN_STATS: [BuiltinStat; 17] = [
    // Current health settles under this ceiling, so a zero would turn any debuff
    // that reached it into an instant kill.
    builtin(StatId::MAX_HEALTH, "max_health", FixedU64::ONE),
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
    builtin(StatId::HEALTH_REGEN, "health_regen", FixedU64::ZERO),
    // Both are fractional rates, so a whole-number floor would raise the values
    // content authors rather than guard them.
    builtin(StatId::REPAIR_SPEED, "repair_speed", FixedU64::ZERO),
    builtin(
        StatId::REPAIR_COST_FACTOR,
        "repair_cost_factor",
        FixedU64::ZERO,
    ),
    // Zero range can only be satisfied by standing inside the target's footprint.
    builtin(StatId::REPAIR_RANGE, "repair_range", FixedU64::ONE),
    builtin(StatId::BUILD_RANGE, "build_range", FixedU64::ONE),
    builtin(StatId::HARVEST_RANGE, "harvest_range", FixedU64::ONE),
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
pub(crate) fn floor_of(stat: StatId) -> FixedU64 {
    match BUILTIN_STATS.get(stat.index()) {
        Some(builtin) => builtin.floor,
        None => FixedU64::ZERO,
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
