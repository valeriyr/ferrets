//! Resource gathering: sources, carriers, and storages.
//! Resource kinds are content-defined strings, not hard-coded in the engine.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// What happens to a source when harvesting empties it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepletionPolicy {
    /// The source dies and leaves the world (a collapsing mine, a felled tree).
    Destroy,
    /// The source stays on the map, empty (an exhausted geyser).
    Persist,
}

/// Content-defined properties of a resource source (gold mine, tree, …).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ResourceSourceStaticData {
    /// The resource kind this source yields.
    kind: String,
    /// What happens to this source when it is emptied.
    depletion: DepletionPolicy,
}

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

/// Where the carrier is during a harvest trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarvestVisibility {
    /// The carrier enters the source and leaves the map for the trip
    /// (a worker inside a gold mine).
    Hidden,
    /// The carrier works in place next to the source (chopping a tree).
    Visible,
}

/// How a carrier harvests one resource kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarvestData {
    /// How much of the kind can be carried at once.
    capacity: u32,
    /// Ticks one harvest trip takes.
    harvest_time: u32,
    /// Where the carrier is during a trip.
    visibility: HarvestVisibility,
}

/// Content-defined harvesting catalogue: which resource kinds this entity can
/// carry, and how it harvests each of them.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ResourceCarrierStaticData {
    carries: BTreeMap<String, HarvestData>,
}

/// Resources a carrier currently holds.
#[derive(Component, Debug, Default)]
pub struct ResourceCarrierComponent {
    /// The carried resource kind. `None` when empty.
    pub kind: Option<String>,
    /// Amount of `kind` currently carried.
    pub amount: u32,
}

/// Content-defined properties of a storage that accepts resource deliveries.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ResourceStorageStaticData {
    /// The resource kinds this storage accepts.
    accepts: Vec<String>,
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

impl ResourceSourceStaticData {
    /// Creates a new `ResourceSourceStaticData` with the given data.
    ///
    /// Panics if `kind` is empty.
    pub fn new(kind: impl Into<String>, depletion: DepletionPolicy) -> Self {
        let kind = kind.into();

        assert!(!kind.is_empty(), "kind must not be empty");

        Self { kind, depletion }
    }

    /// Returns the resource kind this source yields.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Returns what happens to this source when it is emptied.
    pub fn depletion(&self) -> DepletionPolicy {
        self.depletion
    }
}

impl HarvestData {
    /// Creates a new `HarvestData` with the given data.
    ///
    /// Panics if `capacity` or `harvest_time` is `0`.
    pub fn new(capacity: u32, harvest_time: u32, visibility: HarvestVisibility) -> Self {
        assert!(capacity > 0, "capacity must be greater than 0");
        assert!(harvest_time > 0, "harvest_time must be greater than 0");
        Self {
            capacity,
            harvest_time,
            visibility,
        }
    }

    /// Returns how much of the kind can be carried at once.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Returns the duration of one harvest trip in ticks.
    pub fn harvest_time(&self) -> u32 {
        self.harvest_time
    }

    /// Returns where the carrier is during a trip.
    pub fn visibility(&self) -> HarvestVisibility {
        self.visibility
    }
}

impl ResourceCarrierStaticData {
    /// Creates a new `ResourceCarrierStaticData` with the given data.
    ///
    /// Panics if `carries` is empty or contains an empty resource kind.
    pub fn new(carries: impl IntoIterator<Item = (impl Into<String>, HarvestData)>) -> Self {
        let carries: BTreeMap<String, HarvestData> = carries
            .into_iter()
            .map(|(kind, data)| (kind.into(), data))
            .collect();

        assert!(!carries.is_empty(), "carries must not be empty");
        assert!(
            carries.keys().all(|kind| !kind.is_empty()),
            "carried resource kinds must not be empty"
        );

        Self { carries }
    }

    /// Returns `true` if resources of `kind` can be carried.
    pub fn can_carry(&self, kind: &str) -> bool {
        self.carries.contains_key(kind)
    }

    /// Returns the resource kinds that can be carried.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.carries.keys().map(String::as_str)
    }

    /// Returns how `kind` is harvested, or `None` if it cannot be carried.
    pub fn harvest_data(&self, kind: &str) -> Option<&HarvestData> {
        self.carries.get(kind)
    }
}

impl ResourceStorageStaticData {
    /// Creates a new `ResourceStorageStaticData` with the given data.
    ///
    /// Panics if `accepts` is empty or contains an empty resource kind.
    pub fn new(accepts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let accepts: Vec<String> = accepts.into_iter().map(Into::into).collect();

        assert!(!accepts.is_empty(), "accepts must not be empty");
        assert!(
            accepts.iter().all(|kind| !kind.is_empty()),
            "accepted resource kinds must not be empty"
        );

        Self { accepts }
    }

    /// Returns `true` if deliveries of `kind` are accepted here.
    pub fn accepts(&self, kind: &str) -> bool {
        self.accepts.iter().any(|accepted| accepted == kind)
    }

    /// Returns the accepted resource kinds.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.accepts.iter().map(String::as_str)
    }
}
