//! Stores which positions are passable for each movement layer defined by content.

use crate::layer_mask::LayerMask;

pub use crate::layer_id::LayerId;

use super::nav_pos::NavPos;

/// Stores per-layer navigation data for each position in the game map.
#[derive(Debug, Clone)]
pub struct NavGrid {
    width: u32,
    height: u32,
    /// Mask of all registered layers.
    registered: LayerMask,
    /// `occupancy[y * width + x]` — the set of layers occupied at that cell.
    occupancy: Vec<LayerMask>,
}

impl NavGrid {
    /// Creates an empty grid with no layers registered.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            registered: LayerMask::EMPTY,
            occupancy: vec![LayerMask::EMPTY; width as usize * height as usize],
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
    pub fn set_occupied(&mut self, layer: LayerId, pos: NavPos, occupied: bool) {
        self.set_occupied_by(layer, pos, occupied);
    }

    /// Sets whether a position is occupied on all layers matched by `mask`.
    ///
    /// Out-of-bounds positions are silently ignored.
    pub fn set_occupied_by(&mut self, mask: impl Into<LayerMask>, pos: NavPos, occupied: bool) {
        let mask = mask.into();

        self.assert_registered(mask);

        let Some(i) = self.index(pos) else { return };

        if occupied {
            self.occupancy[i] |= mask;
        } else {
            self.occupancy[i] &= !mask;
        }
    }

    /// Returns `true` if the position is occupied on the given layer.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied(&self, layer: LayerId, pos: NavPos) -> bool {
        self.is_occupied_by(layer, pos)
    }

    /// Returns `true` if the position is free on the given layer.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_passable(&self, layer: LayerId, pos: NavPos) -> bool {
        self.is_passable_by(layer, pos)
    }

    /// Returns `true` if the position is occupied on **any** layer in `mask`.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied_by(&self, mask: impl Into<LayerMask>, pos: NavPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.occupancy[i] & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Returns `true` if the position is free on **all** layers in `mask`.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_passable_by(&self, mask: impl Into<LayerMask>, pos: NavPos) -> bool {
        !self.is_occupied_by(mask, pos)
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
    fn index(&self, pos: NavPos) -> Option<usize> {
        if pos.x >= self.width || pos.y >= self.height {
            return None;
        }
        Some(pos.y as usize * self.width as usize + pos.x as usize)
    }
}
