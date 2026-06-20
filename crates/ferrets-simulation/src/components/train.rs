//! In-flight training state and content-defined production properties for
//! simulation entities.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;

/// Content-defined production catalogue: which entity types this entity can train.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct TrainStaticData {
    trains: Vec<String>,
}

impl TrainStaticData {
    /// Creates a new `TrainStaticData` with the given data.
    ///
    /// Panics if `trains` is empty or contains an empty type name.
    pub fn new(trains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let trains: Vec<String> = trains.into_iter().map(Into::into).collect();

        assert!(!trains.is_empty(), "trains must not be empty");
        assert!(
            trains.iter().all(|name| !name.is_empty()),
            "trained type names must not be empty"
        );

        Self { trains }
    }

    /// Returns `true` if units of `type_name` can be trained here.
    pub fn can_train(&self, type_name: &str) -> bool {
        self.trains.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be trained.
    pub fn trains(&self) -> impl Iterator<Item = &str> {
        self.trains.iter().map(String::as_str)
    }
}

/// Pending production, front entry first. Entries are entity type names.
#[derive(Component, Debug, Default)]
pub struct TrainQueueComponent(pub VecDeque<String>);

/// Per-entity in-flight training state.
#[derive(Component, Debug, Default)]
pub struct TrainComponent {
    /// Ticks spent training the front queue entry.
    pub progress: u32,
}
