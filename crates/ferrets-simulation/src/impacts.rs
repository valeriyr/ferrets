//! Damage released by a weapon that has not landed yet.

use bevy_ecs::prelude::*;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::simulation_id::SimulationId;
use ferrets_content::{entity_type_def::EntityTypeId, projectile::ProjectileId};

/// One shot in flight.
///
/// Holds ids and a content handle rather than [`Entity`] references, so a shot
/// outlives the entity that fired it: the firing type's damage bonuses stay
/// readable from the registry after the attacker is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingImpact {
    /// Who fired, recorded on the victim as the damage source.
    pub attacker: SimulationId,
    /// The firing type, for the damage bonuses that outlive the attacker.
    pub attacker_type: EntityTypeId,
    /// Which projectile kind is in the air, so a renderer can tell an arrow from a
    /// cannonball.
    pub projectile: ProjectileId,
    /// The intended victim, for a shot that follows its target. `None` for one aimed
    /// at a cell, which has nobody to follow. Already gone when the shot lands means
    /// the shot was wasted.
    pub target: Option<SimulationId>,
    /// Where the shot was released, for measuring a line blast and for drawing.
    pub origin: FixedUVec2,
    /// Where the shot was aimed. A cell-aimed shot resolves here; a target-following
    /// one recomputes from its target on arrival and uses this only as a fallback.
    pub impact: FixedUVec2,
    /// The attacker's effective damage, frozen when the shot was released.
    pub damage: FixedU64,
    /// The tick the shot was released on.
    pub emitted_on_tick: u32,
    /// The tick the shot lands on.
    pub lands_on_tick: u32,
}

/// Every shot currently in flight, in release order.
///
/// Release order is deterministic because the order loop walks entities in a
/// fixed order, so every peer resolves the same shots in the same sequence.
#[derive(Resource, Debug, Default)]
pub struct PendingImpacts {
    in_flight: Vec<PendingImpact>,
}

impl PendingImpacts {
    /// Queues a shot.
    pub fn push(&mut self, impact: PendingImpact) {
        self.in_flight.push(impact);
    }

    /// Removes and returns every shot due on or before `tick`, in release order.
    pub fn take_due(&mut self, tick: u32) -> Vec<PendingImpact> {
        let (due, pending) = self
            .in_flight
            .iter()
            .partition(|impact| impact.lands_on_tick <= tick);
        self.in_flight = pending;
        due
    }

    /// Every shot still in flight, in release order — for rendering.
    pub fn in_flight(&self) -> &[PendingImpact] {
        &self.in_flight
    }
}
