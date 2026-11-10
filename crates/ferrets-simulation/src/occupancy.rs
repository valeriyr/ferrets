//! How bodies mark the navigation grid.

use ferrets_content::entity_type_def::EntityTypeDef;
use ferrets_geometry::cell_rect::CellRect;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::simulation_id::SimulationId;

/// What a body marks on the navigation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridMark {
    /// The body whose mark it is: the holder of its claims.
    pub holder: SimulationId,
    /// The cells it covers.
    pub footprint: CellRect,
    /// The layers it marks on them.
    pub layers: LayerMask,
    /// The plane it marks them on.
    pub class: OccupancyClass,
}

/// How an entity's footprint occupies the navigation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyClass {
    /// A standing footprint on the static plane — what long-range planning
    /// sees. Placing or clearing one dirties the hierarchy.
    Static,
    /// A mover's claim — honored by movement, invisible to the hierarchy.
    Claim,
}

impl OccupancyClass {
    /// The plane an entity of this definition occupies: movers claim their
    /// cells, everything else stands on the static plane.
    pub fn of(def: &EntityTypeDef) -> Self {
        if def.can_move() {
            OccupancyClass::Claim
        } else {
            OccupancyClass::Static
        }
    }
}
