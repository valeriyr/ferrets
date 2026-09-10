//! The bond between an annex and the primary whose dock it stands in.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;

/// Which primary an annex stands with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Docking {
    /// Nothing standing beside it offers it a dock.
    Alone,
    /// The primary whose dock it stands in.
    Primary(SimulationId),
}

/// Marks a building that stands in another's dock, and says whose.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnexComponent {
    /// The primary it stands with.
    pub docked_to: Docking,
}

impl AnnexComponent {
    /// A newly fitted annex, standing with nobody until the pass that derives
    /// the bond runs.
    pub fn alone() -> Self {
        Self {
            docked_to: Docking::Alone,
        }
    }
}

/// What stands in each of a primary's docks.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct DocksComponent {
    /// The annex in each dock that has one, by the dock's index in the type's
    /// declaration.
    pub annexes: BTreeMap<usize, SimulationId>,
}
