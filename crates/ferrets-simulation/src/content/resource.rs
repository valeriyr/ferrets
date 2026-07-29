//! Content-defined resource property structs: sources, carriers, and storages.

use std::collections::BTreeMap;

/// What happens to a source when harvesting empties it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepletionPolicy {
    /// The source dies and leaves the world (a collapsing mine, a felled tree).
    Destroy,
    /// The source stays on the map, empty (an exhausted geyser).
    Persist,
}

/// Content-defined properties of a resource source (gold mine, tree, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSourceDef {
    /// The resource kind this source yields.
    kind: String,
    /// What happens to this source when it is emptied.
    depletion: DepletionPolicy,
}

impl ResourceSourceDef {
    /// Creates a new `ResourceSourceDef` with the given data.
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

/// Content-defined harvesting catalogue: which resource kinds this entity can
/// carry, and how it harvests each of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCarrierDef {
    carries: BTreeMap<String, HarvestData>,
}

impl ResourceCarrierDef {
    /// Creates a new `ResourceCarrierDef` with the given data.
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

/// Content-defined properties of a storage that accepts resource deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceStorageDef {
    /// The resource kinds this storage accepts.
    accepts: Vec<String>,
}

impl ResourceStorageDef {
    /// Creates a new `ResourceStorageDef` with the given data.
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
