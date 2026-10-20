//! How bodies mark the navigation grid.

use ferrets_content::entity_type_def::EntityTypeDef;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_physics::body;

/// What a body marks on the navigation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridMark {
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

    /// The footprint of `size` a body at `position` holds on this plane,
    /// anchored at the cell the plane anchors a body at.
    pub fn footprint_rect(self, position: FixedUVec2, size: CellSize) -> CellRect {
        CellRect::new(self.anchor(position), size)
    }

    /// The cell a footprint on this plane is anchored at for a body at
    /// `position`: a standing form's stamped anchor, or the cell a mover's
    /// claim lies on.
    fn anchor(self, position: FixedUVec2) -> CellPos {
        match self {
            OccupancyClass::Static => body::anchor(position),
            OccupancyClass::Claim => CellPos::from(position),
        }
    }
}
