//! The live map grid of the active game.

use bevy_ecs::prelude::*;
use ferrets_pathfinder::{
    astar::Projection, nav_grid::NavGrid, nav_pos::NavPos, nav_size::NavSize, search,
};

use crate::components::location::{LocationComponent, LocationStaticData, Solidity};
use crate::map_data::MapData;
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
    /// Where each player starts, ordered by player slot id: player `i` starts at
    /// `start_points[i]`.
    start_points: Vec<NavPos>,
}

impl Map {
    /// Builds the live map a [`MapData`] describes: a fresh grid with the
    /// declared layers registered and nothing occupied yet.
    pub fn from_data(data: &MapData) -> Self {
        let mut nav_grid = NavGrid::new(data.width, data.height);
        for &layer in &data.layers {
            nav_grid.add_layer(layer);
        }
        let start_points = data
            .start_points
            .iter()
            .map(|&(x, y)| NavPos::new(x, y))
            .collect();
        Self::new(data.name.as_str(), data.projection, nav_grid, start_points)
    }

    /// Creates a map from loaded content.
    pub fn new(
        name: impl Into<String>,
        projection: Projection,
        nav_grid: NavGrid,
        start_points: Vec<NavPos>,
    ) -> Self {
        Self {
            name: name.into(),
            projection,
            nav_grid,
            start_points,
        }
    }

    /// Returns every player start position, ordered by player slot id.
    pub fn start_points(&self) -> &[NavPos] {
        &self.start_points
    }

    /// Returns the start position for the given player, or `None` if the map has
    /// no start point for that slot.
    pub fn start_point(&self, player: PlayerId) -> Option<NavPos> {
        self.start_points.get(player as usize).copied()
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
    pub fn can_place_entity(
        &self,
        loc: &LocationComponent,
        static_data: &LocationStaticData,
    ) -> bool {
        self.nav_grid.is_footprint_passable_by(
            static_data.occupation(),
            NavPos::from(loc.position),
            static_data.size(),
        )
    }

    /// Finds a free position for an entity with `spawn_data` properties, scanning
    /// outward from the rectangle of cells at `origin` with the given `size`.
    ///
    /// Cells are scanned ring by ring in row-major order, so the result is
    /// deterministic. Returns `None` when nothing is free within the search radius.
    pub fn find_placement_near(
        &self,
        origin: NavPos,
        size: NavSize,
        spawn_data: &LocationStaticData,
    ) -> Option<NavPos> {
        /// How far out from the rectangle to search before giving up.
        const MAX_RADIUS: u32 = 8;

        search::find_placement_near(
            &self.nav_grid,
            spawn_data.occupation(),
            origin,
            size,
            spawn_data.size(),
            MAX_RADIUS,
        )
    }

    /// Marks every cell in the entity's footprint as occupied.
    pub fn place_entity(&mut self, loc: &LocationComponent, static_data: &LocationStaticData) {
        self.set_footprint(loc, static_data, true);
    }

    /// Clears every cell in the entity's footprint.
    pub fn displace_entity(&mut self, loc: &LocationComponent, static_data: &LocationStaticData) {
        self.set_footprint(loc, static_data, false);
    }

    /// Marks or clears every cell in the entity's footprint as occupied based on the `occupied` parameter.
    ///
    /// No-op for [`Solidity::Passable`] entities — they never claim cells.
    fn set_footprint(
        &mut self,
        loc: &LocationComponent,
        static_data: &LocationStaticData,
        occupied: bool,
    ) {
        if static_data.solidity() == Solidity::Passable {
            return;
        }

        let origin = NavPos::from(loc.position);
        let NavSize { width, height } = static_data.size();
        for dy in 0..height {
            for dx in 0..width {
                self.nav_grid_mut().set_occupied_by(
                    static_data.occupation(),
                    NavPos::new(origin.x + dx, origin.y + dy),
                    occupied,
                );
            }
        }
    }
}
