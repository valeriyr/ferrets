//! Marker for an entity off the grid still waiting for a free cell to return to.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

/// Tags an entity off the grid — hidden, or attached to a job — whose return
/// could not find a free cell near its target footprint, recording the anchor
/// to retry against.
///
/// The return is reattempted each tick around the stored anchor until a cell
/// opens, at which point the entity stands on the grid again and this marker is
/// dropped. It keeps an entity that finished its order while boxed-in from
/// being stranded off the grid.
#[derive(Component, Debug, Clone, Copy)]
pub struct PendingRevealComponent {
    /// Footprint anchor the reveal searches around.
    pub around: CellPos,
    /// Footprint size the reveal searches around.
    pub around_size: CellSize,
}
