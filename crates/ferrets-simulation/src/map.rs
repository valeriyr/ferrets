//! The live map grid of the active game.

use bevy_ecs::prelude::*;
use ferrets_pathfinder::{
    astar::Projection, nav_grid::NavGrid, nav_pos::NavPos, nav_size::NavSize, search,
};

use crate::components::location::LocationComponent;
use crate::content::{
    location::{LocationDef, Solidity},
    registry::ContentRegistry,
};
use crate::map_data::{MapData, MapSlot};
use crate::session::player_slot::PlayerId;

/// The active game map.
#[derive(Resource)]
pub struct Map {
    /// A unique map identifier, e.g. a filename or asset path.
    name: String,
    /// Movement cost model and range metric used across the entire map.
    projection: Projection,
    /// Grid occupancy data. Dimensions define the playable area in cells.
    nav_grid: NavGrid,
    /// Where each player starts, indexed by player slot id; `None` for a seat
    /// with no start position (an environment combatant's).
    start_points: Vec<Option<NavPos>>,
}

impl Map {
    /// Builds the live map a [`MapData`] describes: a fresh grid with every
    /// layer of the registered vocabulary, and each cell blocked on the layers
    /// its terrain is not passable on. A map declaring no terrain starts fully
    /// open; entity occupancy composes on top either way.
    ///
    /// Panics if the terrain palette references an unregistered terrain, a cell
    /// indexes past the palette, or the cell count does not match the grid.
    pub fn from_data(data: &MapData, registry: &ContentRegistry) -> Self {
        let mut nav_grid = NavGrid::new(data.width(), data.height());
        for (_, layer) in registry.layers() {
            nav_grid.add_layer(layer);
        }

        if !data.terrain_cells().is_empty() {
            Self::seed_terrain(&mut nav_grid, data, registry);
        }

        // Start positions indexed by slot id; environment seats have none.
        let start_points = data
            .slots()
            .iter()
            .map(|slot| match slot {
                MapSlot::Player { start: (x, y) } => Some(NavPos::new(*x, *y)),
                MapSlot::Environment => None,
            })
            .collect();
        Self::new(data.name(), data.projection(), nav_grid, start_points)
    }

    /// Marks each cell occupied on the layers its terrain leaves impassable.
    fn seed_terrain(nav_grid: &mut NavGrid, data: &MapData, registry: &ContentRegistry) {
        assert_eq!(
            data.terrain_cells().len(),
            data.width() as usize * data.height() as usize,
            "map '{}' terrain must cover every cell",
            data.name()
        );

        let blocked_per_terrain: Vec<_> = data
            .terrains()
            .iter()
            .map(|name| {
                let passable = registry.terrain(name).unwrap_or_else(|| {
                    panic!("map '{}' uses unregistered terrain '{name}'", data.name())
                });
                registry.registered_layers() & !passable
            })
            .collect();

        for (i, &terrain) in data.terrain_cells().iter().enumerate() {
            let blocked = *blocked_per_terrain
                .get(terrain as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "map '{}' cell {i} indexes past the terrain palette",
                        data.name()
                    )
                });
            let pos = NavPos::new(i as u32 % data.width(), i as u32 / data.width());
            nav_grid.set_occupied_by(blocked, pos, true);
        }
    }

    /// Creates a map from loaded content.
    pub fn new(
        name: impl Into<String>,
        projection: Projection,
        nav_grid: NavGrid,
        start_points: Vec<Option<NavPos>>,
    ) -> Self {
        Self {
            name: name.into(),
            projection,
            nav_grid,
            start_points,
        }
    }

    /// Returns the start position for the given player, or `None` if the map
    /// has none for that slot.
    pub fn start_point(&self, player: PlayerId) -> Option<NavPos> {
        self.start_points.get(player as usize).copied().flatten()
    }

    /// Returns the unique map identifier.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the projection that governs movement costs and range metrics.
    pub fn projection(&self) -> Projection {
        self.projection
    }

    /// Returns the map width in cells.
    pub fn width(&self) -> u32 {
        self.nav_grid.width()
    }

    /// Returns the map height in cells.
    pub fn height(&self) -> u32 {
        self.nav_grid.height()
    }

    /// Returns a reference to the navigation grid.
    pub fn nav_grid(&self) -> &NavGrid {
        &self.nav_grid
    }

    /// Returns a mutable reference to the navigation grid.
    pub fn nav_grid_mut(&mut self) -> &mut NavGrid {
        &mut self.nav_grid
    }

    /// Returns `true` if every cell in the entity's footprint is passable.
    pub fn can_place_entity(&self, loc: &LocationComponent, location_def: &LocationDef) -> bool {
        self.nav_grid.is_footprint_passable_by(
            location_def.occupation(),
            NavPos::from(loc.position),
            location_def.size(),
        )
    }

    /// Finds a free position for an entity with `location_def` properties, scanning
    /// outward from the rectangle of cells at `origin` with the given `size`.
    ///
    /// Cells are scanned ring by ring in row-major order, so the result is
    /// deterministic. Returns `None` when nothing is free within the search radius.
    pub fn find_placement_near(
        &self,
        origin: NavPos,
        size: NavSize,
        location_def: &LocationDef,
    ) -> Option<NavPos> {
        /// How far out from the rectangle to search before giving up.
        const MAX_RADIUS: u32 = 8;

        search::find_placement_near(
            &self.nav_grid,
            location_def.occupation(),
            origin,
            size,
            location_def.size(),
            MAX_RADIUS,
        )
    }

    /// Marks every cell in the entity's footprint as occupied.
    pub fn place_entity(&mut self, loc: &LocationComponent, location_def: &LocationDef) {
        self.set_footprint(loc, location_def, true);
    }

    /// Clears every cell in the entity's footprint.
    pub fn displace_entity(&mut self, loc: &LocationComponent, location_def: &LocationDef) {
        self.set_footprint(loc, location_def, false);
    }

    /// Marks or clears every cell in the entity's footprint as occupied based on the `occupied` parameter.
    ///
    /// No-op for [`Solidity::Passable`] entities — they never claim cells.
    fn set_footprint(
        &mut self,
        loc: &LocationComponent,
        location_def: &LocationDef,
        occupied: bool,
    ) {
        if location_def.solidity() == Solidity::Passable {
            return;
        }

        let origin = NavPos::from(loc.position);
        let NavSize { width, height } = location_def.size();
        for dy in 0..height {
            for dx in 0..width {
                self.nav_grid_mut().set_occupied_by(
                    location_def.occupation(),
                    NavPos::new(origin.x + dx, origin.y + dy),
                    occupied,
                );
            }
        }
    }
}
