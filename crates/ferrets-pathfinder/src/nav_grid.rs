//! Stores which positions are passable for each movement layer defined by content.

use crate::layer_mask::LayerMask;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

pub use crate::layer_id::LayerId;

/// Stores per-layer navigation data for each position in the game map.
///
/// Occupancy lives on two planes: the **static** plane (terrain and standing
/// footprints — what long-range planning honors) and the **claim** plane
/// (cells units hold while resting or crossing — honored by movement, ignored
/// by everything that must not see units).
#[derive(Debug, Clone)]
pub struct NavGrid {
    width: u32,
    height: u32,
    /// Mask of all registered layers.
    registered: LayerMask,
    /// `occupancy[y * width + x]` — the set of layers statically occupied at
    /// that cell.
    occupancy: Vec<LayerMask>,
    /// `claims[y * width + x]` — the set of layers a unit holds at that cell.
    claims: Vec<LayerMask>,
}

impl NavGrid {
    /// Creates an empty grid with no layers registered.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            registered: LayerMask::EMPTY,
            occupancy: vec![LayerMask::EMPTY; width as usize * height as usize],
            claims: vec![LayerMask::EMPTY; width as usize * height as usize],
        }
    }

    /// Returns the grid width in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the grid height in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Registers a fully-open layer.
    ///
    /// Panics if `layer` is already registered.
    pub fn add_layer(&mut self, layer: impl Into<LayerId>) {
        let layer = layer.into();

        assert_eq!(
            self.registered & layer,
            LayerMask::EMPTY,
            "layer {layer} is already registered"
        );

        self.registered |= layer;
    }

    /// Sets whether a position is occupied on the given layer.
    pub fn set_occupied(&mut self, layer: LayerId, pos: CellPos, occupied: bool) {
        self.set_occupied_by(layer, pos, occupied);
    }

    /// Sets whether a position is statically occupied on all layers matched
    /// by `mask`.
    ///
    /// Out-of-bounds positions are silently ignored.
    pub fn set_occupied_by(&mut self, mask: impl Into<LayerMask>, pos: CellPos, occupied: bool) {
        let mask = mask.into();

        self.assert_registered(mask);

        let Some(i) = self.index(pos) else { return };

        if occupied {
            self.occupancy[i] |= mask;
        } else {
            self.occupancy[i] &= !mask;
        }
    }

    /// Sets whether a unit holds the position on all layers matched by
    /// `mask`.
    ///
    /// Out-of-bounds positions are silently ignored.
    pub fn set_claimed_by(&mut self, mask: impl Into<LayerMask>, pos: CellPos, claimed: bool) {
        let mask = mask.into();

        self.assert_registered(mask);

        let Some(i) = self.index(pos) else { return };

        if claimed {
            self.claims[i] |= mask;
        } else {
            self.claims[i] &= !mask;
        }
    }

    /// Returns `true` if the position is occupied on the given layer.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied(&self, layer: LayerId, pos: CellPos) -> bool {
        self.is_occupied_by(layer, pos)
    }

    /// Returns `true` if the position is free on the given layer.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_passable(&self, layer: LayerId, pos: CellPos) -> bool {
        self.is_passable_by(layer, pos)
    }

    /// Returns `true` if the position is occupied — statically or by a unit's
    /// claim — on **any** layer in `mask`.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| (self.occupancy[i] | self.claims[i]) & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Returns `true` if the position is free on **all** layers in `mask`,
    /// counting both static occupancy and unit claims.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_passable_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        !self.is_occupied_by(mask, pos)
    }

    /// Clears every unit claim on every layer, leaving static occupancy
    /// untouched — for rebuilding the claim plane from current positions.
    pub fn clear_claims(&mut self) {
        self.claims.fill(LayerMask::EMPTY);
    }

    /// Returns `true` if a unit holds the position on **any** layer in
    /// `mask`, ignoring static occupancy.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_claimed_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.claims[i] & mask != LayerMask::EMPTY)
            .unwrap_or(false)
    }

    /// Returns `true` if the position is statically occupied on **any** layer
    /// in `mask`, ignoring unit claims.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_statically_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.occupancy[i] & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Returns `true` if the position is statically free on **all** layers in
    /// `mask`, ignoring unit claims.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_statically_passable_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        !self.is_statically_occupied_by(mask, pos)
    }

    /// Returns `true` if every cell of the `size` footprint at `origin` is free
    /// on **all** layers in `mask`.
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn is_footprint_passable_by(
        &self,
        mask: impl Into<LayerMask>,
        origin: CellPos,
        size: CellSize,
    ) -> bool {
        let mask = mask.into();
        let CellSize { width, height } = size;

        for dy in 0..height {
            for dx in 0..width {
                if !self.is_passable_by(mask, CellPos::new(origin.x + dx, origin.y + dy)) {
                    return false;
                }
            }
        }
        true
    }

    /// Panics in debug builds if `mask` contains any unregistered layer bits.
    #[inline]
    fn assert_registered(&self, mask: LayerMask) {
        debug_assert!(
            mask & !self.registered == LayerMask::EMPTY,
            "mask contains unregistered layers"
        );
    }

    /// Converts a grid position to a flat array index, or `None` if out of bounds.
    fn index(&self, pos: CellPos) -> Option<usize> {
        if pos.x >= self.width || pos.y >= self.height {
            return None;
        }
        Some(pos.y as usize * self.width as usize + pos.x as usize)
    }
}
