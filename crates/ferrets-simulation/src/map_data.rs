//! A map described as serializable data, from which a live map is built.

use std::collections::BTreeMap;

use ferrets_geometry::projection::Projection;
use ferrets_nav_grid::{relief::Relief, slope::Slope};
use serde::{Deserialize, Serialize};

use crate::{movement_model::MovementModel, session::player_id::PlayerId};
use ferrets_content::pool::PoolInitial;

/// One entity a map opens with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The entity type to spawn, by content name.
    pub type_name: String,
    /// The cell to spawn it on.
    pub cell: (u32, u32),
    /// The owning slot, or `None` for a neutral entity.
    pub owner: Option<PlayerId>,
    /// Overrides the spawned resource source's starting amount.
    pub amount: Option<u32>,
    /// Where the named pools start, by pool name, overriding the type's.
    pub pools: Vec<(String, PoolInitial)>,
}

/// One seat the map declares; the seat's slot id is its position in the
/// declared list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapSlot {
    /// A player seat, starting at the given cell.
    Player {
        /// The cell the seat's owner starts at.
        start: (u32, u32),
    },
    /// An environment combatant's seat, owning the placements tagged with its
    /// id.
    Environment,
}

/// A map described as data: its grid, the seats it declares, and what stands
/// on it before the first tick.
///
/// Everything player-agnostic about a game's opening scene belongs here —
/// neutral resources are placements with no owner, and owner-tagged placements
/// key their owner by slot id. What a slot's player *does* get at start (a
/// base, a stockpile) is the game's rule, not the map's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapData {
    /// A unique map identifier.
    name: String,
    /// Movement cost model and range metric used across the entire map.
    projection: Projection,
    /// How units occupy space and resolve blocking on this map.
    movement_model: MovementModel,
    /// Playable width in cells.
    width: u32,
    /// Playable height in cells.
    height: u32,
    /// Terrain palette: the registered terrain names the cells reference.
    terrains: Vec<String>,
    /// Per-cell terrain as indices into `terrains`, row-major,
    /// `width × height` entries.
    terrain_cells: Vec<u8>,
    /// Each layer spanning levels' relief, by layer name: per cell, row-major,
    /// `width × height` entries.
    reliefs: BTreeMap<String, Vec<Relief>>,
    /// The seats the map declares, indexed by slot id.
    slots: Vec<MapSlot>,
    /// The entities the map opens with, built in declared order.
    placements: Vec<Placement>,
}

impl MapData {
    /// Creates a `width × height` map with every cell of `terrain`, no
    /// layer reliefs, no seats and no placements.
    ///
    /// Panics if a dimension is zero.
    pub fn new(
        name: impl Into<String>,
        projection: Projection,
        width: u32,
        height: u32,
        terrain: impl Into<String>,
    ) -> Self {
        assert!(
            width > 0 && height > 0,
            "map dimensions must be greater than 0"
        );
        Self {
            name: name.into(),
            projection,
            movement_model: MovementModel::default(),
            width,
            height,
            terrains: vec![terrain.into()],
            terrain_cells: vec![0; width as usize * height as usize],
            reliefs: BTreeMap::new(),
            slots: Vec::new(),
            placements: Vec::new(),
        }
    }

    /// Returns the unique map identifier.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the movement cost model and range metric used across the map.
    pub fn projection(&self) -> Projection {
        self.projection
    }

    /// Returns how units occupy space and resolve blocking on this map.
    pub fn movement_model(&self) -> MovementModel {
        self.movement_model
    }

    /// Selects how units occupy space and resolve blocking on this map,
    /// replacing the default of [`MovementModel::Cell`].
    pub fn set_movement_model(&mut self, model: MovementModel) {
        self.movement_model = model;
    }

    /// Replaces the distance metric the map was constructed with.
    pub fn set_projection(&mut self, projection: Projection) {
        self.projection = projection;
    }

    /// Returns the playable width in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the playable height in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Returns the terrain palette: the registered terrain names the cells
    /// reference.
    pub fn terrains(&self) -> &[String] {
        &self.terrains
    }

    /// Returns the per-cell terrain as indices into
    /// [`terrains`](Self::terrains), row-major, `width × height` entries.
    pub fn terrain_cells(&self) -> &[u8] {
        &self.terrain_cells
    }

    /// Returns a layer's relief, per cell, row-major, `width × height`
    /// entries, or `None` where the map gives the layer none.
    pub fn relief(&self, layer: &str) -> Option<&[Relief]> {
        self.reliefs.get(layer).map(Vec::as_slice)
    }

    /// Returns every layer relief the map gives, by layer name, in name order.
    pub fn reliefs(&self) -> impl Iterator<Item = (&str, &[Relief])> {
        self.reliefs
            .iter()
            .map(|(layer, relief)| (layer.as_str(), relief.as_slice()))
    }

    /// Returns the seats the map declares, indexed by slot id.
    pub fn slots(&self) -> &[MapSlot] {
        &self.slots
    }

    /// Returns each player seat's slot id and start cell, in slot-id order.
    pub fn player_seats(&self) -> impl Iterator<Item = (PlayerId, (u32, u32))> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| match slot {
                MapSlot::Player { start } => Some((id as PlayerId, *start)),
                MapSlot::Environment => None,
            })
    }

    /// Returns each environment seat's slot id, in slot-id order.
    pub fn environment_seats(&self) -> impl Iterator<Item = PlayerId> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| match slot {
                MapSlot::Environment => Some(id as PlayerId),
                MapSlot::Player { .. } => None,
            })
    }

    /// Returns the entities the map opens with, in declared order.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// Sets the terrain of one cell, adding the name to the palette if new.
    ///
    /// Panics if the cell is out of bounds, or the palette overflows.
    pub fn set_terrain(&mut self, cell: (u32, u32), terrain: &str) {
        let cell_index = self.cell_index(cell);

        let index = self
            .terrains
            .iter()
            .position(|name| name == terrain)
            .unwrap_or_else(|| {
                self.terrains.push(terrain.to_string());
                self.terrains.len() - 1
            });
        let index = u8::try_from(index).expect("terrain palette overflow");
        self.terrain_cells[cell_index] = index;
    }

    /// Gives `layer` a relief with every cell at `level` and no slopes.
    ///
    /// Panics if the map gives the layer a relief already.
    pub fn add_relief(&mut self, layer: &str, level: u32) {
        let cells = self.width as usize * self.height as usize;
        let relief = vec![Relief { level, slope: None }; cells];
        assert!(
            self.reliefs.insert(layer.to_string(), relief).is_none(),
            "map '{}' gives layer '{layer}' a relief twice",
            self.name
        );
    }

    /// Sets `layer`'s level at one cell.
    ///
    /// Panics if the cell is out of bounds or the map gives the layer no
    /// relief.
    pub fn set_cell_level(&mut self, layer: &str, cell: (u32, u32), level: u32) {
        self.relief_at(layer, cell).level = level;
    }

    /// Makes one cell part of a slope on `layer`.
    ///
    /// Panics if the cell is out of bounds or the map gives the layer no
    /// relief.
    pub fn set_slope(&mut self, layer: &str, cell: (u32, u32), slope: Slope) {
        self.relief_at(layer, cell).slope = Some(slope);
    }

    /// Declares the next seat as a player seat starting at `cell`, returning
    /// its slot id.
    ///
    /// Panics if the cell is out of bounds.
    pub fn add_player_slot(&mut self, cell: (u32, u32)) -> PlayerId {
        let (x, y) = cell;
        assert!(
            x < self.width && y < self.height,
            "player seat start ({x}, {y}) is out of bounds"
        );
        self.slots.push(MapSlot::Player { start: cell });
        (self.slots.len() - 1) as PlayerId
    }

    /// Declares the next seat as an environment combatant's, returning its
    /// slot id.
    pub fn add_environment_slot(&mut self) -> PlayerId {
        self.slots.push(MapSlot::Environment);
        (self.slots.len() - 1) as PlayerId
    }

    /// Appends an entity the map opens with.
    ///
    /// Panics if the placement's cell is out of bounds, or its owner is not a
    /// declared seat — declare the seats first.
    pub fn add_placement(&mut self, placement: Placement) {
        let (x, y) = placement.cell;
        assert!(
            x < self.width && y < self.height,
            "placement '{}' cell ({x}, {y}) is out of bounds",
            placement.type_name
        );
        if let Some(owner) = placement.owner {
            assert!(
                (owner as usize) < self.slots.len(),
                "placement '{}' owner {owner} is not a declared seat",
                placement.type_name
            );
        }
        self.placements.push(placement);
    }

    /// `layer`'s relief at `cell`.
    ///
    /// Panics if the cell is out of bounds or the map gives the layer no
    /// relief.
    fn relief_at(&mut self, layer: &str, cell: (u32, u32)) -> &mut Relief {
        let index = self.cell_index(cell);
        let name = &self.name;
        &mut self
            .reliefs
            .get_mut(layer)
            .unwrap_or_else(|| panic!("map '{name}' gives layer '{layer}' no relief"))[index]
    }

    /// The row-major index of `cell`.
    ///
    /// Panics if the cell is out of bounds.
    fn cell_index(&self, cell: (u32, u32)) -> usize {
        let (x, y) = cell;
        assert!(
            x < self.width && y < self.height,
            "cell ({x}, {y}) is out of bounds"
        );
        (y * self.width + x) as usize
    }
}
