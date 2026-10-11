//! Marker for entities that are temporarily off the map.

use bevy_ecs::prelude::*;

use crate::components::location::LocationComponent;

/// Marks an entity that is temporarily not present on the map — a builder inside
/// its construction site, a carrier inside a mine, a passenger inside a
/// transport, or a body waiting on a free cell to come back to.
///
/// A hidden entity has no [`LocationComponent`], does not occupy the navigation
/// grid, cannot be selected, and cannot be targeted.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenComponent {
    /// Where the entity stood when it was taken off the map.
    pub went_in: LocationComponent,
}

impl HiddenComponent {
    /// Creates a new `HiddenComponent` with the given data.
    #[inline]
    pub fn new(went_in: LocationComponent) -> Self {
        Self { went_in }
    }
}
