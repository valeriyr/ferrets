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

use crate::layer_mask::LayerMask;

/// The geometry a route must fit: the layers a mover occupies and the size of
/// the footprint it stamps wherever it stands.
///
/// Deliberately anchor-free — a shape becomes a footprint only once a search
/// anchors it at a candidate cell, and the same shape is anchored at every
/// cell a route tests. Two movers with the same shape route identically,
/// which is what lets one hierarchy abstraction and one shared plan serve a
/// whole group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MoverShape {
    /// The layers every cell of the footprint must be free on.
    pub mask: LayerMask,
    /// The size of the footprint that must fit at each anchor tested.
    pub size: CellSize,
}

impl MoverShape {
    /// Creates a shape from the layers a mover occupies and its footprint size.
    pub fn new(mask: impl Into<LayerMask>, size: CellSize) -> Self {
        Self {
            mask: mask.into(),
            size,
        }
    }

    /// A single-cell shape on `mask` — the shape of everything that covers one
    /// cell, and of intermediate goals a search aims at.
    pub fn point(mask: impl Into<LayerMask>) -> Self {
        Self::new(mask, CellSize::ONE)
    }
}
