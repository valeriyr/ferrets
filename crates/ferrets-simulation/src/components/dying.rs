//! Death-phase markers for simulation entities.

use bevy_ecs::prelude::*;

/// Marks an entity that is in the process of dying.
///
/// A dying entity cannot be selected and does not accept new orders, but it
/// still holds its footprint on the navigation grid.
#[derive(Component, Debug)]
pub struct DyingComponent {
    /// Ticks left until the entity finishes dying and is removed from the world.
    pub ticks_remaining: u32,
}

/// Marks remains spawned directly into the dying state (a corpse, rubble).
///
/// Distinguishes the two meanings of a running [`DyingComponent`]: without this
/// marker the entity is transitioning out of the alive world (a death
/// animation); with it, the entity is remains whose dying timer is its entire
/// existence (decay).
#[derive(Component, Debug, Default)]
pub struct CorpseComponent;

/// Marks an entity whose dying phase has completed and is ready to be despawned.
#[derive(Component, Debug, Default)]
pub struct DiedComponent;
