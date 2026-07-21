//! Health state and content-defined health properties for simulation entities.

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;

/// Content-defined health properties for an entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthStaticData {
    /// Maximum health points.
    max_health: u32,
}

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
    current: u32,
    /// The most recent damage source, if the entity has ever been hit.
    last_hit: Option<LastHit>,
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
