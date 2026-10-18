//! Content-defined production-catalogue property struct.

use crate::{field::FieldPlacement, landing::Landing};

/// Where a trainer's units come into the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    /// Beside the trainer, once the queue's train time has run.
    Beside,
    /// On a cell the player picks, at once, rising over the unit's own
    /// `rise`; the trainer cools down for the unit's train time.
    Warped {
        /// The field rules the picked cells must meet for the trainer's owner.
        within: Vec<FieldPlacement>,
        /// Which cells the owner's side must know of to warp onto.
        landing: Landing,
    },
}

/// Content-defined production catalogue: which entity types this entity can train.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainerDef {
    /// The types it trains, by name.
    trains: Vec<String>,
    /// Where what it trains comes into the world.
    release: Release,
}

impl TrainerDef {
    /// Creates a new `TrainerDef` with the given data.
    ///
    /// Panics if `trains` is empty or contains an empty type name.
    pub fn new(trains: impl IntoIterator<Item = impl Into<String>>, release: Release) -> Self {
        let trains: Vec<String> = trains.into_iter().map(Into::into).collect();

        assert!(!trains.is_empty(), "trains must not be empty");
        assert!(
            trains.iter().all(|name| !name.is_empty()),
            "trained type names must not be empty"
        );

        Self { trains, release }
    }

    /// Where what it trains comes into the world.
    pub fn release(&self) -> &Release {
        &self.release
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
