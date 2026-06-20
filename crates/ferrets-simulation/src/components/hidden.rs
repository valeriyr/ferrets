//! Visibility marker for entities that are temporarily off the map.

use bevy_ecs::prelude::*;

/// Marks an entity that is temporarily not present on the map — a builder inside
/// its construction site, a carrier inside a mine, a passenger inside a transport.
///
/// A hidden entity does not occupy the navigation grid, cannot be selected, and
/// cannot be targeted.
#[derive(Component, Debug, Default)]
pub struct HiddenComponent;
