//! In-flight attack state and content-defined combat properties for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

/// Content-defined combat properties for an entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackStaticData {
    /// Health points removed from the target by one hit.
    damage: u32,
    /// Maximum distance to the target in grid cells. `1` means adjacent cells only.
    range: u32,
    /// Ticks from the start of a swing until the hit lands.
    aiming: u32,
    /// Ticks after a hit before the next swing can start.
    reloading: u32,
}

/// Per-entity in-flight attack state.
#[derive(Component, Debug, Default)]
pub struct AttackComponent {
    /// Current position inside the swing cycle. Counts up each tick: the hit lands
    /// when it reaches `aiming`, and the cycle restarts at `aiming + reloading`.
    pub phase: u32,
    /// `(own position, target position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}

impl AttackStaticData {
    /// Creates a new `AttackStaticData` with the given data.
    ///
    /// Panics if `aiming` or `reloading` is `0`.
    #[inline]
    pub fn new(damage: u32, range: u32, aiming: u32, reloading: u32) -> Self {
        assert!(aiming > 0, "aiming must be greater than 0");
        assert!(reloading > 0, "reloading must be greater than 0");
        Self {
            damage,
            range,
            aiming,
            reloading,
        }
    }

    /// Returns the health points removed from the target by one hit.
    #[inline]
    pub fn damage(&self) -> u32 {
        self.damage
    }

    /// Returns the maximum distance to the target in grid cells.
    #[inline]
    pub fn range(&self) -> u32 {
        self.range
    }

    /// Returns the number of ticks until a swing lands its hit.
    #[inline]
    pub fn aiming(&self) -> u32 {
        self.aiming
    }

    /// Returns the number of ticks after a hit before the next swing.
    #[inline]
    pub fn reloading(&self) -> u32 {
        self.reloading
    }
}
