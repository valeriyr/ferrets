//! Stores, for each cell of the game map, its terrain and what occupies it on
//! each layer content defines.

use crate::{
    layer_mask::LayerMask,
    mover_profile::{Blockers, MoverProfile},
    mover_shape::MoverShape,
    terrain_id::TerrainId,
    terrain_mask::TerrainMask,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

pub use crate::layer_id::LayerId;

/// Stores per-cell navigation data for the game map.
///
/// Each cell has a **terrain**, fixed at map build, and occupancy on two
/// planes per layer: the **occupancy** plane (standing footprints) and the
/// **claim** plane (cells units hold while resting or crossing). Terrain and
/// occupancy together are the **static** plane long-range planning honors;
/// claims are honored by movement and ignored by everything that must not
/// see units.
#[derive(Debug, Clone)]
pub struct NavGrid {
    width: u32,
    height: u32,
    /// Mask of all registered layers.
    registered: LayerMask,
    /// `terrain[y * width + x]` — the cell's terrain.
    terrain: Vec<TerrainId>,
    /// `occupancy[y * width + x]` — the set of layers a standing footprint
    /// occupies at that cell.
    occupancy: Vec<LayerMask>,
    /// `claims[y * width + x]` — the set of layers a unit holds at that cell.
    claims: Vec<LayerMask>,
}

impl NavGrid {
    /// Creates a grid with no layers registered, each cell of the terrain
    /// `terrain_at` gives it.
    pub fn new(width: u32, height: u32, terrain_at: impl Fn(CellPos) -> TerrainId) -> Self {
        Self {
            width,
            height,
            registered: LayerMask::EMPTY,
            terrain: CellRect::new(CellPos::new(0, 0), CellSize::new(width, height))
                .cells()
                .map(terrain_at)
                .collect(),
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

    /// Returns a position's terrain, or `None` off the grid.
    pub fn terrain(&self, pos: CellPos) -> Option<TerrainId> {
        self.index(pos).map(|i| self.terrain[i])
    }

    /// Sets whether a standing footprint occupies a position on all layers
    /// matched by `mask`; terrain is untouched either way.
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

    /// Returns `true` if the position is occupied — by a standing footprint or
    /// a unit's claim — on **any** layer in `mask`, whatever its terrain.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| (self.occupancy[i] | self.claims[i]) & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Clears every unit claim on every layer, leaving terrain and standing
    /// occupancy untouched — for rebuilding the claim plane from current positions.
    pub fn clear_claims(&mut self) {
        self.claims.fill(LayerMask::EMPTY);
    }

    /// Returns `true` if a unit holds the position on **any** layer in
    /// `mask`, ignoring standing footprints.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_claimed_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.claims[i] & mask != LayerMask::EMPTY)
            .unwrap_or(false)
    }

    /// Returns `true` if a standing footprint occupies the position on
    /// **any** layer in `mask`, ignoring unit claims and terrain.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_statically_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.occupancy[i] & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Returns `true` if the position's terrain is one of `terrains`, whatever
    /// stands or rests on it.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_on_terrains(&self, terrains: TerrainMask, pos: CellPos) -> bool {
        self.terrain(pos)
            .is_some_and(|terrain| terrains.meets(terrain.into()))
    }

    /// Returns `true` if the position is on one of `terrains` and free of
    /// standing occupancy and unit claims alike on `layers`.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn cell_fits(&self, pos: CellPos, terrains: TerrainMask, layers: LayerMask) -> bool {
        self.is_on_terrains(terrains, pos) && !self.is_occupied_by(layers, pos)
    }

    /// Returns `true` if the position is on one of `terrains` and free of
    /// standing occupancy on `layers`, whatever units claim.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn cell_fits_statically(
        &self,
        pos: CellPos,
        terrains: TerrainMask,
        layers: LayerMask,
    ) -> bool {
        self.is_on_terrains(terrains, pos) && !self.is_statically_occupied_by(layers, pos)
    }

    /// Returns `true` if every cell of the shape's footprint at `anchor`
    /// [`cell_fits`](Self::cell_fits) its terrains and layers.
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn fits(&self, anchor: CellPos, shape: MoverShape) -> bool {
        CellRect::new(anchor, shape.size)
            .cells()
            .all(|cell| self.cell_fits(cell, shape.terrains, shape.layers))
    }

    /// Returns `true` if every cell of the shape's footprint at `anchor`
    /// [`cell_fits_statically`](Self::cell_fits_statically) its terrains and
    /// layers.
    ///
    /// This is the clearance question long-range planning asks: whether a mover
    /// of that shape could ever stand here, as opposed to whether it can right
    /// now. Footprints reaching out of bounds always return `false`.
    pub fn fits_statically(&self, anchor: CellPos, shape: MoverShape) -> bool {
        CellRect::new(anchor, shape.size)
            .cells()
            .all(|cell| self.cell_fits_statically(cell, shape.terrains, shape.layers))
    }

    /// Returns `true` if every cell of the `size` footprint at `anchor` is on one
    /// of `terrains`, whoever else is there.
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn fits_terrain(&self, anchor: CellPos, terrains: TerrainMask, size: CellSize) -> bool {
        CellRect::new(anchor, size)
            .cells()
            .all(|cell| self.is_on_terrains(terrains, cell))
    }

    /// Returns `true` if the profile's footprint at `anchor` is open
    /// under its blocker mode.
    pub(crate) fn fits_for(&self, anchor: CellPos, profile: MoverProfile) -> bool {
        match profile.blockers {
            Blockers::All => self.fits(anchor, profile.shape),
            Blockers::Static => self.fits_statically(anchor, profile.shape),
        }
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
