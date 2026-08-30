//! A weapon: the non-scalar properties of what a body or a turret fires.
//!
//! The measurements — damage, ranges, cadence, the arc it fires through and how
//! fast it comes to bear — are scalar stats living in the mounting type's base
//! stats, so the modifier pipeline can move them. What groups here is what no
//! modifier can touch: what a weapon reaches, how its hit travels, and what that
//! hit spreads over. Where a weapon is carried is [`crate::turret`]'s business,
//! except for the one a body points itself, which is [`AttackDef`].

use ferrets_pathfinder::layer_mask::LayerMask;

use crate::{projectile::ProjectileId, splash::SplashDef};

/// How a hit reaches what it was aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// It lands in the tick the cycle emits it, however far off the target
    /// stands.
    Instant,
    /// It travels as the given projectile kind and lands when it arrives.
    Projectile(ProjectileId),
}

/// A weapon: what it reaches, how its hit travels, and what that hit spreads
/// over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weapon {
    /// The navigation layers it can reach — see
    /// [`targeting::reaches`](crate::targeting::reaches).
    targets: LayerMask,
    /// How its hit reaches what it was aimed at.
    delivery: Delivery,
    /// What its hit spreads over, or `None` for one that damages only what it
    /// hit. Independent of the delivery: a cleaving swing lands instantly and
    /// still catches everything beside it.
    splash: Option<SplashDef>,
}

impl Weapon {
    /// Creates a new `Weapon` with the given data.
    ///
    /// Panics if `targets` is empty, which would leave it unable to hit anything
    /// at all.
    pub fn new(
        targets: impl Into<LayerMask>,
        delivery: Delivery,
        splash: Option<SplashDef>,
    ) -> Self {
        let targets = targets.into();
        assert!(
            targets != LayerMask::EMPTY,
            "a weapon's targets must not be empty"
        );
        Self {
            targets,
            delivery,
            splash,
        }
    }

    /// The navigation layers it can reach.
    #[inline]
    pub fn targets(&self) -> LayerMask {
        self.targets
    }

    /// How its hit reaches what it was aimed at.
    #[inline]
    pub fn delivery(&self) -> Delivery {
        self.delivery
    }

    /// What its hit spreads over.
    #[inline]
    pub fn splash(&self) -> Option<&SplashDef> {
        self.splash.as_ref()
    }
}

/// What a type's body brings to a fight: the weapon it points itself.
///
/// One field today. It stays a struct of its own because what a body does with a
/// weapon is not finished being described — where it may fire from, what it does
/// while reloading — and none of that belongs to the weapon, which a turret
/// carries just as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackDef {
    /// The weapon the body points.
    weapon: Weapon,
}

impl AttackDef {
    /// Creates a new `AttackDef` with the given data.
    pub fn new(weapon: Weapon) -> Self {
        Self { weapon }
    }

    /// The weapon the body points.
    #[inline]
    pub fn weapon(&self) -> &Weapon {
        &self.weapon
    }
}
