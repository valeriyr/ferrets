//! Stores which positions are passable for each movement layer defined by content.

use bevy::prelude::*;

use super::nav_pos::NavPos;

/// Identifies a movement layer.
pub type LayerId = u32;

/// Stores per-layer navigation data for each position in the game map.
#[derive(Resource, Debug, Clone)]
pub struct NavGrid {
    width: u32,
    height: u32,
    /// Bitmask of all registered layer IDs.
    registered: u32,
    /// `occupancy[y * width + x]` — each set bit indicates an occupied layer.
    occupancy: Vec<u32>,
}

impl NavGrid {
    /// Creates an empty grid with no layers registered.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            registered: 0,
            occupancy: vec![0; (width * height) as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Registers a fully-open layer with the given ID.
    ///
    /// `layer` must be a non-zero power of two (e.g. 1, 2, 4, 8 …) so it can serve as its own bitmask.
    pub fn add_layer(&mut self, layer: LayerId) {
        assert!(
            layer > 0 && layer.is_power_of_two(),
            "layer must be a non-zero power of two"
        );
        assert_eq!(
            self.registered & layer,
            0,
            "layer {layer} is already registered"
        );
        self.registered |= layer;
    }

    /// Sets whether a position is occupied on the given layer.
    pub fn set_occupied(&mut self, layer: LayerId, pos: NavPos, occupied: bool) {
        self.set_occupied_by(layer, pos, occupied);
    }

    /// Sets whether a position is occupied on all layers matched by `mask`.
    pub fn set_occupied_by(&mut self, mask: u32, pos: NavPos, occupied: bool) {
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
    pub fn is_occupied_by(&self, mask: u32, pos: NavPos) -> bool {
        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.occupancy[i] & mask != 0)
            .unwrap_or(true)
    }

    /// Returns `true` if the position is free on **all** layers in `mask`.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_passable_by(&self, mask: u32, pos: NavPos) -> bool {
        !self.is_occupied_by(mask, pos)
    }

    /// Panics in debug builds if `mask` contains any unregistered layer bits.
    #[inline]
    fn assert_registered(&self, mask: u32) {
        debug_assert_eq!(
            mask & !self.registered,
            0,
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
