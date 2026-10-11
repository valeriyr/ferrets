//! Marker for an entity off the grid still waiting for a free cell to return to.

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_rect::CellRect;

use crate::beside::{Beside, Lean};

/// Tags an entity off the grid still waiting for a free cell to return to.
///
/// The return is reattempted each tick until a cell opens, at which point the
/// entity stands on the grid again and this marker is dropped; an entity
/// carrying it is `Operation::Stranded`.
#[derive(Component, Debug, Clone, Copy)]
pub struct PendingRevealComponent {
    /// What the return searches beside, read again on every attempt.
    pub beside: Beside,
    /// Which cell beside it the return takes first.
    pub lean: Lean,
    /// The footprint the return last searched beside; it stands in for a
    /// container gone from the world. Where the entity went in, for a
    /// container already gone when the return was deferred.
    pub last: CellRect,
}
