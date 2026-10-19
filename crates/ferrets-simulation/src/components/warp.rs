//! The live state of an entity that warps units in.

use bevy_ecs::prelude::*;

/// A warper's live state: when it may warp in its next unit.
#[derive(Component, Debug, Default)]
pub struct WarpComponent {
    /// The earliest tick it may warp in its next unit at.
    pub ready_at: u32,
}
