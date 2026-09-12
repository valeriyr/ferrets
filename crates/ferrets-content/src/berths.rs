//! Content-defined berths: the points about a footprint an attached worker may
//! sit at, and how many workers they seat.

use std::collections::BTreeMap;

use ferrets_math::fixed_vec2::FixedVec2;

/// One berth group: the points its workers sit at and move between, and how
/// many workers it seats at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BerthGroup {
    /// Where a worker's middle sits, as signed offsets in cells from the
    /// footprint's anchor, inside the footprint or around it; also the spots a
    /// moving worker crosses between. Each holds one worker at a time.
    points: Vec<FixedVec2>,
    /// How many workers the group seats at once, never more than it has
    /// points.
    slots: usize,
}

impl BerthGroup {
    /// Creates a new `BerthGroup` with the given data.
    ///
    /// Panics if `points` is empty, or `slots` is `0` or more than the points.
    pub fn new(points: impl IntoIterator<Item = FixedVec2>, slots: usize) -> Self {
        let points: Vec<FixedVec2> = points.into_iter().collect();
        assert!(
            !points.is_empty(),
            "a berth group must have at least one point"
        );
        assert!(slots > 0, "a berth group must seat at least one worker");
        assert!(
            slots <= points.len(),
            "a berth group cannot seat more workers than it has points"
        );
        Self { points, slots }
    }

    /// The points workers sit at.
    #[inline]
    pub fn points(&self) -> &[FixedVec2] {
        &self.points
    }

    /// How many workers the group seats at once.
    #[inline]
    pub fn slots(&self) -> usize {
        self.slots
    }
}

/// Named berth groups on one entity type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BerthsDef {
    /// The groups, by name.
    groups: BTreeMap<String, BerthGroup>,
}

impl BerthsDef {
    /// Creates a new `BerthsDef` with the given data.
    ///
    /// Panics if `groups` is empty or a group name is empty.
    pub fn new(groups: impl IntoIterator<Item = (impl Into<String>, BerthGroup)>) -> Self {
        let groups: BTreeMap<String, BerthGroup> = groups
            .into_iter()
            .map(|(name, group)| (name.into(), group))
            .collect();

        assert!(!groups.is_empty(), "berth groups must not be empty");
        assert!(
            groups.keys().all(|name| !name.is_empty()),
            "berth group names must not be empty"
        );

        Self { groups }
    }

    /// The group named `group`, or `None` when no such group is declared.
    pub fn group(&self, group: &str) -> Option<&BerthGroup> {
        self.groups.get(group)
    }

    /// Every group, in name order.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &BerthGroup)> {
        self.groups
            .iter()
            .map(|(name, group)| (name.as_str(), group))
    }
}
