//! Timed life: how long an instance has stood, for the types that do not stand
//! indefinitely.

use bevy_ecs::prelude::*;

/// How long an entity has stood, in ticks.
///
/// Fitted to instances of types carrying the `lifetime` stat; when the age
/// reaches that stat's effective value the entity's time is up. The age counts
/// up rather than down so a buff that lengthens or shortens the stat acts on
/// instances already standing.
#[derive(Component, Debug, Default)]
pub struct LifetimeComponent {
    /// Ticks the entity has stood.
    pub age: u32,
}
