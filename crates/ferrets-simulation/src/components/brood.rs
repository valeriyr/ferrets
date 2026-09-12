//! Breeding state: a breeder's timer and the broodlings it counts, and the tie
//! a bred entity keeps to its breeder.

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;

/// A breeder's timer and the broodlings it counts.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct BroodComponent {
    /// Ticks toward the next birth.
    pub progress: u32,
    /// Births owed to bring the brood up to what the form worn opens with,
    /// delivered as soon as the breeder breeds.
    pub owed: u32,
    /// The broodlings alive and counted, in the order they joined the brood.
    pub broodlings: Vec<SimulationId>,
}

/// The tie a bred entity keeps to its breeder.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct BredComponent {
    /// The breeder.
    pub by: SimulationId,
}
