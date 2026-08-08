//! Marker for a hidden entity still waiting for a free cell to reappear on.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

/// Tags a hidden entity whose reveal could not find a free cell near its target
/// footprint, recording the anchor to retry against.
///
/// The reveal is reattempted each tick around the stored anchor until a cell
/// opens, at which point the entity reappears and this marker is dropped. It
/// keeps an entity that finished its order while boxed-in from being stranded
/// off the map.
#[derive(Component, Debug, Clone, Copy)]
pub struct PendingRevealComponent {
    /// Footprint origin the reveal searches around.
    pub around: CellPos,
    /// Footprint size the reveal searches around.
    pub around_size: CellSize,
}
