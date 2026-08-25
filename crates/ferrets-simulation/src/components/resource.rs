//! Resource-gathering runtime state: source amounts, carrier loads, and harvest progress.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;
use crate::simulation_id::SimulationId;

/// Remaining resources in a source.
///
/// Starts at `0`; the actual amount is per-instance state set when the map is
/// populated, not part of the entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceSourceComponent {
    pub amount: u32,
}

/// The crew working a source, present exactly while somebody is on a trip there.
#[derive(Component, Debug, Default)]
pub struct UnderHarvestComponent {
    /// The carriers on a trip to the source right now.
    pub carriers: BTreeSet<SimulationId>,
}

/// Marks a carrier at work on a source.
///
/// Present for the duration of a harvest trip, whatever the carrier's declared
/// presence: whether it can be seen while it works is
/// [`crate::components::hidden::HiddenComponent`]'s answer, not this one's.
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
#[derive(Component, Debug)]
pub struct HarvestComponent {
    /// Ticks spent on the current harvest trip.
    pub progress: u32,
    /// The source currently being worked, if a trip is in progress.
    pub harvesting: Option<SimulationId>,
    /// The source this order settled on, kept across deliveries so the carrier
    /// returns to it instead of searching again.
    pub source: Option<SimulationId>,
    /// The resource kind this order harvests, so a wood order does not drift
    /// to gold. Named by the order's target or the load in hand and never
    /// reassigned; an order that can name no kind has nothing to harvest and
    /// never starts.
    pub kind: String,
    /// Ticks left standing in place before a walk the carrier could not finish
    /// — to a source or to a storage — is retried.
    pub wait: u32,
    /// Set once at least one trip has completed for this order; a storage-targeted
    /// order delivers the current load first, then keeps harvesting.
    pub delivered_initial_load: bool,
    /// The last chase round toward the source or storage being walked to;
    /// identical rounds accumulate until the chase gives up (see
    /// [`ChaseState`]).
    pub last_chase: ChaseState,
}

impl HarvestComponent {
    /// Creates in-flight harvest state for an order collecting `kind`, settled
    /// on `source` when the order targeted one.
    pub fn new(kind: String, source: Option<SimulationId>) -> Self {
        Self {
            progress: 0,
            harvesting: None,
            source,
            kind,
            wait: 0,
            delivered_initial_load: false,
            last_chase: None,
        }
    }
}
