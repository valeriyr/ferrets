//! Turrets: guns a body carries that point where they like.
//!
//! A turret is content of its own, defined once and mounted by the types that
//! carry it, the way a projectile is. What it is — the weapon, and which of the
//! mounting type's stats each of its numbers reads — belongs to the definition;
//! where it sits belongs to the body that mounts it.
//!
//! Nothing here knows about the body's own weapon. A body weapon points along the
//! body and so stops to shoot; a turret has a bearing of its own and so need not.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

use crate::{attack::Weapon, entity_stats::EntityStatId};

/// A registered turret's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurretId(usize);

impl TurretId {
    /// Creates a handle for the turret registered at `index`.
    #[inline]
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    /// The registration index this handle names.
    #[inline]
    pub fn index(self) -> usize {
        self.0
    }
}

/// What a turret asks of the body while it fights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponConduct {
    /// It works a target only while the body stands: a fight and a walk are not
    /// had at once.
    Halts,
    /// It works a target while the body goes about its orders.
    OnTheMove,
}

/// Which of the mounting type's stats each of a turret's numbers reads.
///
/// The numbers themselves stay entity stats, so the modifier pipeline reaches
/// them as it always has, and a turret carried by two types reads each type's own
/// values. Two turrets on one body fight by different numbers by naming different
/// stats — content declares its own with `define_entity_stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurretStats {
    /// What one landed hit takes off.
    pub damage: EntityStatId,
    /// How far it shoots.
    pub range: EntityStatId,
    /// How far it engages on its own initiative.
    pub acquire_range: EntityStatId,
    /// Ticks in one full cycle.
    pub period: EntityStatId,
    /// The tick within the cycle its hit leaves on.
    pub damage_point: EntityStatId,
    /// How far it comes round in a tick.
    pub aim_rate: EntityStatId,
    /// How far off its bearing it may still fire.
    pub arc: EntityStatId,
}

impl Default for TurretStats {
    /// The stats a gun reads unless it says otherwise.
    fn default() -> Self {
        Self {
            damage: EntityStatId::DAMAGE,
            range: EntityStatId::ATTACK_RANGE,
            acquire_range: EntityStatId::ACQUIRE_RANGE,
            period: EntityStatId::ATTACK_PERIOD,
            damage_point: EntityStatId::DAMAGE_POINT,
            aim_rate: EntityStatId::AIM_RATE,
            arc: EntityStatId::ATTACK_ARC,
        }
    }
}

/// A turret: a weapon with a bearing of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurretDef {
    /// The weapon it fires.
    weapon: Weapon,
    /// Which stats its numbers read.
    stats: TurretStats,
    /// What it asks of the body while it fights.
    conduct: WeaponConduct,
}

impl TurretDef {
    /// Creates a new `TurretDef` with the given data.
    pub fn new(weapon: Weapon, stats: TurretStats, conduct: WeaponConduct) -> Self {
        Self {
            weapon,
            stats,
            conduct,
        }
    }

    /// The weapon it fires.
    #[inline]
    pub fn weapon(&self) -> &Weapon {
        &self.weapon
    }

    /// Which stats its numbers read.
    #[inline]
    pub fn stats(&self) -> TurretStats {
        self.stats
    }

    /// What it asks of the body while it fights.
    #[inline]
    pub fn conduct(&self) -> WeaponConduct {
        self.conduct
    }
}

/// A turret as one body carries it: which gun, and the patch of the body it sits
/// on.
///
/// The patch is a rectangle rather than a point because everything measured here
/// is: its middle is where the shot leaves from, and a drawing that wants to put
/// the gun somewhere has the room it takes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurretMount {
    /// The gun mounted.
    turret: TurretId,
    /// Where it sits, in cells from the mounting footprint's own origin.
    origin: CellPos,
    /// How much of that footprint it takes up.
    size: CellSize,
}

impl TurretMount {
    /// Creates a new `TurretMount` with the given data.
    pub fn new(turret: TurretId, origin: CellPos, size: CellSize) -> Self {
        Self {
            turret,
            origin,
            size,
        }
    }

    /// The gun mounted.
    #[inline]
    pub fn turret(&self) -> TurretId {
        self.turret
    }

    /// Where it sits, in cells from the mounting footprint's own origin.
    #[inline]
    pub fn origin(&self) -> CellPos {
        self.origin
    }

    /// How much of that footprint it takes up.
    #[inline]
    pub fn size(&self) -> CellSize {
        self.size
    }
}

/// How the turrets one body carries divide their own targets between them.
///
/// It governs only what they find for themselves: an order names a target for the
/// whole body, and every gun that can reach it works it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurretFire {
    /// Each gun takes the target it would take alone, so they agree and a target
    /// under a body's guns is under all of them at once.
    #[default]
    Focus,
    /// A gun passes over what another on the same body already holds, while there
    /// is anything else to take — and falls back to the held one when there is
    /// not, so a lone attacker is still worked by every gun that bears.
    Spread,
}
