//! Content-defined projectile property struct.

use ferrets_math::FixedU64;

/// A handle to a registered projectile kind, assigned in registration order.
///
/// Content declares projectile kinds by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer. A shot carries its kind, which is how a renderer tells an arrow
/// from a cannonball.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectileId(u16);

impl ProjectileId {
    /// Creates a projectile id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more projectiles registered than ProjectileId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a projectile's hit resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    /// The entity itself: the hit follows it, and a blast centres wherever it is when
    /// the hit lands, so it never misses.
    Entity,
    /// A cell, fixed when the shot was released: the hit resolves there whatever has
    /// happened since, so a target that keeps moving escapes it.
    Position,
}

/// Content-defined delivery of a weapon's damage as a travelling projectile.
///
/// A type without this definition delivers damage in the same tick the attack
/// cycle reaches its damage point. With it, the damage point releases a
/// projectile instead, and the hit lands after the flight time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectileDef {
    /// Flight speed in cells per tick. The flight time is the distance to the
    /// target's footprint divided by this, rounded up to whole ticks — measured with
    /// the map's own range metric, so like every range stat it is comparable only
    /// within one projection.
    speed: FixedU64,
    /// What the hit resolves against when it lands.
    aim: Aim,
}

impl ProjectileDef {
    /// Creates a new `ProjectileDef` with the given data.
    ///
    /// Panics if `speed` is zero — a projectile that never arrives would leave
    /// the damage permanently pending.
    pub fn new(speed: FixedU64, aim: Aim) -> Self {
        assert!(speed > FixedU64::ZERO, "projectile speed must be positive");

        Self { speed, aim }
    }

    /// Returns the flight speed in cells per tick.
    #[inline]
    pub fn speed(&self) -> FixedU64 {
        self.speed
    }

    /// Returns what the hit resolves against.
    #[inline]
    pub fn aim(&self) -> Aim {
        self.aim
    }
}
