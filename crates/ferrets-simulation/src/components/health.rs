//! Health state and content-defined health properties for simulation entities.

use bevy_ecs::prelude::*;

/// Content-defined health properties for an entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthStaticData {
    /// Maximum health points.
    max_health: u32,
}

/// Current health of an entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthComponent {
    /// Remaining health points. `0` means the entity is dead.
    current: u32,
}

impl HealthStaticData {
    /// Creates a new `HealthStaticData` with the given data.
    ///
    /// Panics if `max_health` is `0`.
    #[inline]
    pub fn new(max_health: u32) -> Self {
        assert!(max_health > 0, "max_health must be greater than 0");
        Self { max_health }
    }

    /// Returns the maximum health points.
    #[inline]
    pub fn max_health(&self) -> u32 {
        self.max_health
    }
}

impl HealthComponent {
    /// Creates a `HealthComponent` at full health.
    #[inline]
    pub fn full(static_data: &HealthStaticData) -> Self {
        Self {
            current: static_data.max_health(),
        }
    }

    /// Returns the remaining health points.
    #[inline]
    pub fn current(&self) -> u32 {
        self.current
    }

    /// Returns `true` when health has reached `0`.
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.current == 0
    }

    /// Reduces health by `amount`, saturating at `0`.
    #[inline]
    pub fn apply_damage(&mut self, amount: u32) {
        self.current = self.current.saturating_sub(amount);
    }
}
