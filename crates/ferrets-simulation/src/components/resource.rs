//! Resource-gathering runtime state: source amounts, carrier loads, and harvest progress.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// Remaining resources in a source.
///
/// Starts at `0`; the actual amount is per-instance state set when the map is
/// populated, not part of the entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceSourceComponent {
    pub amount: u32,
}

/// Marks a source that a carrier is currently harvesting. Other carriers wait.
#[derive(Component, Debug, Default)]
pub struct UnderHarvestComponent;

/// Marks a carrier that is working a source in place.
///
/// Present for the duration of a visible harvest trip; hidden trips mark the
/// carrier with [`crate::components::hidden::HiddenComponent`] instead.
#[derive(Component, Debug, Default)]
pub struct HarvestingComponent;

/// Resources a carrier currently holds.
#[derive(Component, Debug, Default)]
pub struct ResourceCarrierComponent {
    /// The carried resource kind. `None` when empty.
    pub kind: Option<String>,
    /// Amount of `kind` currently carried.
    pub amount: u32,
}

/// Per-entity in-flight harvest state.
#[derive(Component, Debug, Default)]
pub struct HarvestComponent {
    /// Ticks spent on the current harvest trip.
    pub progress: u32,
    /// The source currently being worked, if a trip is in progress.
    pub harvesting: Option<SimulationId>,
    /// The last source worked; harvesting resumes here after a delivery.
    pub source: Option<SimulationId>,
    /// Set once at least one trip has completed for this order; a storage-targeted
    /// order delivers the current load first, then keeps harvesting.
    pub delivered_initial_load: bool,
    /// `(own position, destination position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
