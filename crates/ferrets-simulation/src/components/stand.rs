//! Whether an entity has yet to perform what its type does on standing.

use bevy_ecs::prelude::*;

/// Where an entity stands with the acts its type performs on standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// The acts are still to be performed, the first tick the entity stands
    /// finished.
    Pending,
    /// The acts have been performed; they never repeat.
    Done,
}

/// Carried by every entity whose type declares acts on standing.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandComponent(pub Standing);
