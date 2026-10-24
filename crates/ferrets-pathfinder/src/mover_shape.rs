//! The shape a route is planned for.
//!
//! Clearance is asked in these terms. A cell counts as open for a mover only
//! when the whole footprint anchored at it is open, so a two-wide body simply
//! does not see one-wide gaps — dilation asked per cell rather than
//! materialized into a second grid. Asking rather than storing keeps one
//! source of truth: there is no dilated plane to invalidate alongside the
//! occupancy it was derived from.
//!
//! Paths are therefore sequences of **anchor** cells, consistent with how a
//! standing footprint is anchored, and never sequences of cells the mover's
//! middle passes over.

use ferrets_geometry::cell_size::CellSize;

use crate::{
    cliff_crossing::CliffCrossing, layer_id::LayerId, layer_mask::LayerMask,
    terrain_mask::TerrainMask,
};

/// The geometry a route must fit.
///
/// Deliberately anchor-free — a shape becomes a footprint only once a search
/// anchors it at a candidate cell, and the same shape is anchored at every
/// cell a route tests. Two movers with the same shape route identically,
/// which is what lets one hierarchy abstraction and one shared plan serve a
/// whole group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MoverShape {
    /// The terrains every cell of the footprint must be on.
    pub terrains: TerrainMask,
    /// The layer whose levels the mover stands and moves on.
    pub layer: LayerId,
    /// The layers every cell of the footprint must be free on.
    pub layers: LayerMask,
    /// The size of the footprint that must fit at each anchor tested.
    pub size: CellSize,
    /// How the mover crosses a cliff, or `None` when its layer is at one
    /// level.
    pub cliff_crossing: Option<CliffCrossing>,
}

impl MoverShape {
    /// Creates a shape from the terrains a mover may be over, the layer it
    /// moves on, the layers it occupies, its footprint size and how it crosses
    /// a cliff.
    pub fn new(
        terrains: TerrainMask,
        layer: LayerId,
        layers: impl Into<LayerMask>,
        size: CellSize,
        cliff_crossing: Option<CliffCrossing>,
    ) -> Self {
        Self {
            terrains,
            layer,
            layers: layers.into(),
            size,
            cliff_crossing,
        }
    }

    /// A single-cell shape over `terrains` moving on and occupying `layer`,
    /// crossing a cliff as `cliff_crossing` — the shape of everything that
    /// covers one cell, and of intermediate goals a search aims at.
    pub fn point(
        terrains: TerrainMask,
        layer: LayerId,
        cliff_crossing: Option<CliffCrossing>,
    ) -> Self {
        Self::new(terrains, layer, layer, CellSize::ONE, cliff_crossing)
    }
}
