//! The entity-stat vocabulary: handles and the engine's built-in entity stats.
//!
//! One stat group among its peers: the typed id is a gate keeping this group's
//! handles apart from the others', while the shared machinery in
//! [`super::stats`] folds them all the same way.

use ferrets_math::FixedU64;

use crate::content::stats::{self, BuiltinStat};

/// A handle to a registered entity stat, assigned in registration order.
///
/// The built-in entity stats occupy the low ids given by the associated
/// constants; content-declared entity stats follow in registration order.
/// Identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityStatId(u16);

impl EntityStatId {
    /// Maximum health points. Current health is runtime state on the health
    /// component, not a stat.
    pub const MAX_HEALTH: EntityStatId = EntityStatId(0);
    /// Health points removed from a target by one hit, before armor.
    pub const DAMAGE: EntityStatId = EntityStatId(1);
    /// Flat damage subtracted from each incoming hit.
    pub const ARMOR: EntityStatId = EntityStatId(2);
    /// Movement speed in grid units per tick. Fractional, and authored below `1`
    /// for most entities, so it carries no floor: zero is a meaningful value that
    /// immobilises the entity, and a walk simply holds without advancing until the
    /// debuff lifts.
    pub const SPEED: EntityStatId = EntityStatId(3);
    /// Map-reveal radius in cells.
    pub const SIGHT_RANGE: EntityStatId = EntityStatId(4);
    /// Attack range in cells.
    pub const ATTACK_RANGE: EntityStatId = EntityStatId(5);
    /// Distance in cells at which enemies are engaged on the entity's own initiative.
    pub const ACQUIRE_RANGE: EntityStatId = EntityStatId(6);
    /// Ticks in one full attack cycle — the rate of fire (`DPS = damage / attack_period`).
    pub const ATTACK_PERIOD: EntityStatId = EntityStatId(7);
    /// Ticks into the attack cycle at which the hit lands (at most `attack_period`).
    pub const DAMAGE_POINT: EntityStatId = EntityStatId(8);
    /// Maximum energy available to spend on skills.
    pub const MAX_ENERGY: EntityStatId = EntityStatId(9);
    /// Energy regenerated per tick, toward [`MAX_ENERGY`](Self::MAX_ENERGY).
    pub const ENERGY_REGEN: EntityStatId = EntityStatId(10);
    /// Health regenerated per tick, toward [`MAX_HEALTH`](Self::MAX_HEALTH).
    pub const HEALTH_REGEN: EntityStatId = EntityStatId(11);
    /// How fast one worker mends, as a multiple of the target's production rate:
    /// `1` restores the target in the time it takes to produce one.
    pub const REPAIR_SPEED: EntityStatId = EntityStatId(12);
    /// Share of a target's own cost charged for mending it in full — `0.25` bills a
    /// quarter of the price to restore an empty pool.
    pub const REPAIR_COST_FACTOR: EntityStatId = EntityStatId(13);
    /// How close a mender must be to its work, in cells.
    pub const REPAIR_RANGE: EntityStatId = EntityStatId(14);
    /// How close a builder must be to a site it is raising, in cells.
    pub const BUILD_RANGE: EntityStatId = EntityStatId(15);
    /// How close a carrier must be to a source it works, in cells.
    pub const HARVEST_RANGE: EntityStatId = EntityStatId(16);
    /// Supply headroom instances add to their owner's pool while standing.
    pub const SUPPLY_PROVIDED: EntityStatId = EntityStatId(17);
    /// Supply instances occupy in their owner's pool, from the moment they are
    /// queued for training.
    pub const SUPPLY_COST: EntityStatId = EntityStatId(18);
    /// The radius of an instance's circular body in cells, where the
    /// continuous movement model resolves contact. Every mover defines one;
    /// half a cell makes resting neighbors touch at one-cell spacing.
    pub const RADIUS: EntityStatId = EntityStatId(19);

    /// Creates an entity stat id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more entity stats registered than EntityStatId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The built-in entity stats, registered first and in this order, so their
/// assigned ids equal the [`EntityStatId`] constants above.
pub(crate) const ENTITY_BUILTIN_STATS: [BuiltinStat<EntityStatId>; 20] = [
    // Current health settles under this ceiling, so a zero would turn any debuff
    // that reached it into an instant kill.
    stats::builtin(EntityStatId::MAX_HEALTH, "max_health", FixedU64::ONE),
    stats::builtin(EntityStatId::DAMAGE, "damage", FixedU64::ZERO),
    stats::builtin(EntityStatId::ARMOR, "armor", FixedU64::ZERO),
    // No floor: speed is fractional grid units per tick, and authored values sit
    // below 1, so any whole-number floor would raise them instead of guarding them.
    stats::builtin(EntityStatId::SPEED, "speed", FixedU64::ZERO),
    stats::builtin(EntityStatId::SIGHT_RANGE, "sight_range", FixedU64::ZERO),
    // Zero range can only be satisfied by standing inside the target's footprint.
    stats::builtin(EntityStatId::ATTACK_RANGE, "attack_range", FixedU64::ONE),
    stats::builtin(EntityStatId::ACQUIRE_RANGE, "acquire_range", FixedU64::ZERO),
    // The attack cycle counts 1..=period and the hit lands on the damage point, so
    // a zero for either is a phase the counter never reaches.
    stats::builtin(EntityStatId::ATTACK_PERIOD, "attack_period", FixedU64::ONE),
    stats::builtin(EntityStatId::DAMAGE_POINT, "damage_point", FixedU64::ONE),
    stats::builtin(EntityStatId::MAX_ENERGY, "max_energy", FixedU64::ZERO),
    stats::builtin(EntityStatId::ENERGY_REGEN, "energy_regen", FixedU64::ZERO),
    stats::builtin(EntityStatId::HEALTH_REGEN, "health_regen", FixedU64::ZERO),
    // Both are fractional rates, so a whole-number floor would raise the values
    // content authors rather than guard them.
    stats::builtin(EntityStatId::REPAIR_SPEED, "repair_speed", FixedU64::ZERO),
    stats::builtin(
        EntityStatId::REPAIR_COST_FACTOR,
        "repair_cost_factor",
        FixedU64::ZERO,
    ),
    // Zero range can only be satisfied by standing inside the target's footprint.
    stats::builtin(EntityStatId::REPAIR_RANGE, "repair_range", FixedU64::ONE),
    stats::builtin(EntityStatId::BUILD_RANGE, "build_range", FixedU64::ONE),
    stats::builtin(EntityStatId::HARVEST_RANGE, "harvest_range", FixedU64::ONE),
    stats::builtin(
        EntityStatId::SUPPLY_PROVIDED,
        "supply_provided",
        FixedU64::ZERO,
    ),
    stats::builtin(EntityStatId::SUPPLY_COST, "supply_cost", FixedU64::ZERO),
    // Fractional cells, authored below 1 — a whole-number floor would raise
    // the values rather than guard them.
    stats::builtin(EntityStatId::RADIUS, "radius", FixedU64::ZERO),
];

// Floors and names are looked up by `EntityStatId::index`, so every entry must sit at
// the slot its own id names.
const _: () = {
    let mut index = 0;
    while index < ENTITY_BUILTIN_STATS.len() {
        assert!(ENTITY_BUILTIN_STATS[index].id.index() == index);
        index += 1;
    }
};

/// The smallest effective value the entity stat at registration `index` may
/// fold to. Content-declared entity stats carry no engine meaning, so they have
/// no floor beyond the non-negative clamp.
pub(crate) fn floor_of(index: usize) -> FixedU64 {
    match ENTITY_BUILTIN_STATS.get(index) {
        Some(builtin) => builtin.floor,
        None => FixedU64::ZERO,
    }
}
