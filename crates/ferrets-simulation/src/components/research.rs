//! In-flight research state for simulation entities.

use bevy_ecs::prelude::*;

use crate::content::research::ResearchId;

/// Per-entity in-flight research state.
#[derive(Component, Debug)]
pub struct ResearchComponent {
    /// The research being worked on.
    pub research: ResearchId,
    /// Ticks spent so far.
    pub progress: u32,
}
