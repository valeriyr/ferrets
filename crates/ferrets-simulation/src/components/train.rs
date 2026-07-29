//! In-flight training state for simulation entities.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;

/// Pending production, front entry first. Entries are entity type names.
#[derive(Component, Debug, Default)]
pub struct TrainQueueComponent(pub VecDeque<String>);

/// Per-entity in-flight training state.
#[derive(Component, Debug, Default)]
pub struct TrainComponent {
    /// Ticks spent training the front queue entry.
    pub progress: u32,
}
