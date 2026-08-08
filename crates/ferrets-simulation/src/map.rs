//! The live map grid of the active game.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_pathfinder::{
    hierarchy::{DEFAULT_CLUSTER_SIZE, NavHierarchy},
    layer_mask::LayerMask,
    nav_grid::NavGrid,
    search,
};

use ferrets_math::FixedU64;

use crate::{
    components::location::LocationComponent,
    content::{
        entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::LocationDef,
        registry::ContentRegistry,
    },
    map_data::{MapData, MapSlot},
    movement_model::MovementModel,
    session::player_slot::PlayerId,
};

/// How an entity's footprint occupies the navigation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyClass {
    /// A standing footprint on the static plane — what long-range planning
    /// sees. Placing or clearing one dirties the hierarchy.
    Static,
    /// A mover's claim — honored by movement, invisible to the hierarchy.
    Claim,
}

impl OccupancyClass {
    /// The plane an entity of this definition occupies: movers claim their
    /// cells, everything else stands on the static plane.
    pub fn of(def: &EntityTypeDef) -> Self {
        if def.can_move() {
            OccupancyClass::Claim
        } else {
            OccupancyClass::Static
        }
    }
}

/// The active game map.
#[derive(Resource)]
pub struct Map {
    /// A unique map identifier, e.g. a filename or asset path.
    name: String,
    /// Movement cost model and range metric used across the entire map.
    projection: Projection,
    /// How units occupy space and resolve blocking on this map.
    movement_model: MovementModel,
    /// Grid occupancy data. Dimensions define the playable area in cells.
    nav_grid: NavGrid,
    /// The hierarchical view of the grid's static plane, one abstraction per
    /// mover mask.
    hierarchy: NavHierarchy,
    /// Where each player starts, indexed by player slot id; `None` for a seat
    /// with no start position (an environment combatant's).
    start_points: Vec<Option<CellPos>>,
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
                MapSlot::Player { start: (x, y) } => Some(CellPos::new(*x, *y)),
                MapSlot::Environment => None,
            })
            .collect();
        // A continuous game resolves contact between bodies, so every mover
        // must author one; a cell game never reads the stat.
        match data.movement_model() {
            MovementModel::Cell => {}
            MovementModel::Continuous => {
                for def in registry.entities() {
                    if !def.can_move() {
                        continue;
                    }
                    let radius = def.base_stats.get(&EntityStatId::RADIUS);
                    assert!(
                        radius.is_some_and(|radius| *radius > FixedU64::ZERO),
                        "entity type '{}' moves but defines no positive radius, \
                         which the continuous movement model requires",
                        def.name
                    );
                }
            }
        }

        // The hierarchy serves one abstraction per distinct mover mask the
        // content declares; the registry iterates deterministically.
        let mut mover_masks: Vec<LayerMask> = Vec::new();
        for def in registry.entities() {
            if let Some(location) = def.location
                && def.can_move()
                && !mover_masks.contains(&location.occupation())
            {
                mover_masks.push(location.occupation());
            }
        }

        Self::with_hierarchy_masks(
            data.name(),
            data.projection(),
            data.movement_model(),
            nav_grid,
            start_points,
            &mover_masks,
        )
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
            let pos = CellPos::new(i as u32 % data.width(), i as u32 / data.width());
            nav_grid.set_occupied_by(blocked, pos, true);
        }
    }

    /// Creates a map from loaded content, without any hierarchy abstractions.
    pub fn new(
        name: impl Into<String>,
        projection: Projection,
        movement_model: MovementModel,
        nav_grid: NavGrid,
        start_points: Vec<Option<CellPos>>,
    ) -> Self {
        Self::with_hierarchy_masks(
            name,
            projection,
            movement_model,
            nav_grid,
            start_points,
            &[],
        )
    }

    /// Creates a map from loaded content, building one hierarchy abstraction
    /// per given mover mask.
    pub fn with_hierarchy_masks(
        name: impl Into<String>,
        projection: Projection,
        movement_model: MovementModel,
        nav_grid: NavGrid,
        start_points: Vec<Option<CellPos>>,
        mover_masks: &[LayerMask],
    ) -> Self {
        let hierarchy = NavHierarchy::build(&nav_grid, DEFAULT_CLUSTER_SIZE, mover_masks);
        Self {
            name: name.into(),
            projection,
            movement_model,
            nav_grid,
            hierarchy,
            start_points,
        }
    }

    /// Returns a reference to the hierarchical view of the grid's static
    /// plane.
    pub fn hierarchy(&self) -> &NavHierarchy {
        &self.hierarchy
    }

    /// Folds every static occupancy change since the last call into the
    /// hierarchy. Called at one fixed point of the game tick.
    pub fn refresh_hierarchy(&mut self) {
        self.hierarchy.refresh(&self.nav_grid);
    }

    /// Returns the start position for the given player, or `None` if the map
    /// has none for that slot.
    pub fn start_point(&self, player: PlayerId) -> Option<CellPos> {
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

    /// Returns how units occupy space and resolve blocking on this map.
    pub fn movement_model(&self) -> MovementModel {
        self.movement_model
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
            CellPos::from(loc.position),
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
        origin: CellPos,
        size: CellSize,
        location_def: &LocationDef,
    ) -> Option<CellPos> {
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

    /// Marks every cell in the entity's footprint as occupied on the plane
    /// its class selects.
    pub fn place_entity(
        &mut self,
        loc: &LocationComponent,
        location_def: &LocationDef,
        class: OccupancyClass,
    ) {
        self.set_footprint(loc, location_def, class, true);
    }

    /// Clears every cell in the entity's footprint on the plane its class
    /// selects.
    pub fn displace_entity(
        &mut self,
        loc: &LocationComponent,
        location_def: &LocationDef,
        class: OccupancyClass,
    ) {
        self.set_footprint(loc, location_def, class, false);
    }

    /// Marks or clears every cell in the entity's footprint as occupied based on the `occupied` parameter.
    ///
    /// Static footprints dirty the hierarchy; claims never do. No-op for
    /// [`Solidity::Passable`] entities — they never claim cells.
    fn set_footprint(
        &mut self,
        loc: &LocationComponent,
        location_def: &LocationDef,
        class: OccupancyClass,
        occupied: bool,
    ) {
        if !location_def.solidity().claims_cells() {
            return;
        }

        let origin = CellPos::from(loc.position);
        let CellSize { width, height } = location_def.size();
        for dy in 0..height {
            for dx in 0..width {
                let cell = CellPos::new(origin.x + dx, origin.y + dy);
                match class {
                    OccupancyClass::Static => {
                        self.nav_grid
                            .set_occupied_by(location_def.occupation(), cell, occupied);
                        self.hierarchy.mark_dirty(cell);
                    }
                    OccupancyClass::Claim => {
                        // One claimant per cell per layer is the cell
                        // model's contract; the continuous model rebuilds
                        // the plane from bodies each tick and legally
                        // collapses shared cells into one bit — and its
                        // clears belong to that rebuild alone: a footprint
                        // release keyed on the floored anchor could wipe a
                        // neighbor's center-cell claim mid-tick.
                        match self.movement_model {
                            MovementModel::Cell => {
                                debug_assert!(
                                    self.nav_grid.is_claimed_by(location_def.occupation(), cell)
                                        != occupied,
                                    "a claim write must flip the cell: claiming needs it \
                                     free, releasing needs it held"
                                );
                            }
                            MovementModel::Continuous => {
                                if !occupied {
                                    continue;
                                }
                            }
                        }
                        self.nav_grid
                            .set_claimed_by(location_def.occupation(), cell, occupied);
                    }
                }
            }
        }
    }
}
