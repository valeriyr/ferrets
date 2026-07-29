//! Energy: the per-entity resource pool skills spend, regenerating each tick.

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

/// Current energy of an entity — the resource skills spend, regenerating toward
/// the entity's maximum (the `max_energy` stat) by the `energy_regen` stat per tick.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergyComponent {
    current: FixedU64,
}

impl EnergyComponent {
    /// Creates an energy pool filled to `max`.
    pub fn full(max: FixedU64) -> Self {
        Self { current: max }
    }

    /// The current energy.
    pub fn current(&self) -> FixedU64 {
        self.current
    }

    /// The current energy as a whole number (truncated) — for display and
    /// integer-only consumers.
    #[inline]
    pub fn current_as_u32(&self) -> u32 {
        self.current.to_num::<u32>()
    }

    /// Spends `cost` if affordable, returning `true` on success.
    pub fn spend(&mut self, cost: FixedU64) -> bool {
        if self.current >= cost {
            self.current -= cost;
            true
        } else {
            false
        }
    }

    /// Regenerates one tick's worth of energy, capped at `max`.
    pub fn regenerate(&mut self, regen: FixedU64, max: FixedU64) {
        self.current = (self.current + regen).min(max);
    }
}
