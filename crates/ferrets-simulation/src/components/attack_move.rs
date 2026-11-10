//! In-flight attack-move state for simulation entities.

use bevy_ecs::prelude::*;

/// What an attack-move is doing between its own turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackMoveStage {
    /// Choosing: no sub-order runs.
    Deciding,
    /// A walk toward the destination runs; its end is the order's.
    Walking,
    /// A fight runs; the walk resumes after it.
    Fighting,
}

/// Per-entity in-flight attack-move state.
#[derive(Component, Debug)]
pub struct AttackMoveComponent {
    /// What the order is suspended into, or deciding what comes next.
    pub stage: AttackMoveStage,
}

impl AttackMoveComponent {
    /// An attack-move deciding what to do first, with no walk behind it.
    pub fn deciding() -> Self {
        Self {
            stage: AttackMoveStage::Deciding,
        }
    }
}
