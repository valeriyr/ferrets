//! Health state for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use crate::simulation_id::SimulationId;

/// The most recent damage source: who hit the entity and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastHit {
    /// The entity that dealt the damage.
    pub attacker: SimulationId,
    /// The tick the hit landed on.
    pub tick: u32,
}

/// Current health of an entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthComponent {
    /// Remaining health points. `0` means the entity is dead.
    current: FixedU64,
    /// The most recent damage source, if the entity has ever been hit.
    last_hit: Option<LastHit>,
}

impl HealthComponent {
    /// Creates a `HealthComponent` at full health, given the maximum health.
    #[inline]
    pub fn full(max: FixedU64) -> Self {
        Self {
            current: max,
            last_hit: None,
        }
    }

    /// Returns the most recent damage source, if the entity has ever been hit.
    #[inline]
    pub fn last_hit(&self) -> Option<LastHit> {
        self.last_hit
    }

    /// Records the source of a hit that just landed.
    #[inline]
    pub fn record_hit(&mut self, attacker: SimulationId, tick: u32) {
        self.last_hit = Some(LastHit { attacker, tick });
    }

    /// Returns the remaining health points.
    #[inline]
    pub fn current(&self) -> FixedU64 {
        self.current
    }

    /// Health points for display and integer-only consumers: `0` exactly when
    /// dead, otherwise at least `1` — a barely-alive entity never reads as `0`.
    #[inline]
    pub fn displayed(&self) -> u32 {
        if self.current == FixedU64::ZERO {
            0
        } else {
            self.current.to_num::<u32>().max(1)
        }
    }

    /// Returns `true` when health has reached `0`.
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.current == FixedU64::ZERO
    }

    /// Reduces health by `amount`, saturating at `0`.
    #[inline]
    pub fn apply_damage(&mut self, amount: FixedU64) {
        self.current = self.current.saturating_sub(amount);
    }

    /// Restores `amount` health, capped at `max`.
    #[inline]
    pub fn heal(&mut self, amount: FixedU64, max: FixedU64) {
        self.current = (self.current + amount).min(max);
    }
}
