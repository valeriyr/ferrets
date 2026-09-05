//! A map described as serializable data, from which a live map is built.

use ferrets_geometry::projection::Projection;
use serde::{Deserialize, Serialize};

use crate::{movement_model::MovementModel, session::player_id::PlayerId};

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
    /// `width × height` entries. Empty means the map declares no terrain and
    /// every cell is passable on every layer.
    terrain_cells: Vec<u8>,
    /// The seats the map declares, indexed by slot id.
    slots: Vec<MapSlot>,
    /// The entities the map opens with, built in declared order.
    placements: Vec<Placement>,
}

impl MapData {
    /// Creates an empty `width × height` map: no terrain declared (every cell
    /// passable on every layer), no seats, no placements.
    ///
    /// Panics if a dimension is zero.
    pub fn new(name: impl Into<String>, projection: Projection, width: u32, height: u32) -> Self {
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
            terrains: Vec::new(),
            terrain_cells: Vec::new(),
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
    /// Empty means the map declares no terrain and every cell is passable on
    /// every layer.
    pub fn terrain_cells(&self) -> &[u8] {
        &self.terrain_cells
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

    /// Declares the map's terrain by setting every cell to `terrain`,
    /// replacing whatever terrain was declared before. Overwrite individual
    /// cells afterwards with [`set_terrain`](Self::set_terrain).
    pub fn fill_terrain(&mut self, terrain: impl Into<String>) {
        self.terrains = vec![terrain.into()];
        self.terrain_cells = vec![0; self.width as usize * self.height as usize];
    }

    /// Sets the terrain of one cell, adding the name to the palette if new.
    ///
    /// Panics if no terrain has been declared (see
    /// [`fill_terrain`](Self::fill_terrain)), the cell is out of bounds, or
    /// the palette overflows.
    pub fn set_terrain(&mut self, cell: (u32, u32), terrain: &str) {
        assert!(
            !self.terrain_cells.is_empty(),
            "declare the map's terrain with fill_terrain before setting cells"
        );
        let (x, y) = cell;
        assert!(
            x < self.width && y < self.height,
            "cell ({x}, {y}) is out of bounds"
        );

        let index = self
            .terrains
            .iter()
            .position(|name| name == terrain)
            .unwrap_or_else(|| {
                self.terrains.push(terrain.to_string());
                self.terrains.len() - 1
            });
        let index = u8::try_from(index).expect("terrain palette overflow");
        self.terrain_cells[(y * self.width + x) as usize] = index;
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
}
