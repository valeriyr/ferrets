//! The live map grid of the active game.

use bevy_ecs::prelude::*;
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_pathfinder::{
    hierarchy::{DEFAULT_CLUSTER_SIZE, NavHierarchy},
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    search,
};

use ferrets_math::FixedU64;

use crate::{
    components::location::LocationComponent,
    map_data::{MapData, MapSlot},
    movement_model::MovementModel,
    session::player_slot::PlayerId,
};
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::LocationDef,
    registry::ContentRegistry,
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
    /// mover shape.
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
                    // The planner clears routes for the footprint; a body
                    // circle wider than the footprint's inscribed circle
                    // pokes into cells no plan ever cleared and walls
                    // itself mid-corridor.
                    let size = def
                        .location
                        .expect("validated content defines a location for movers")
                        .size();
                    let bound = FixedU64::from_num(size.width.min(size.height)) / 2;
                    assert!(
                        radius.is_some_and(|radius| *radius <= bound),
                        "entity type '{}' authors a radius beyond half its \
                         footprint's narrow side ({bound}), which the \
                         continuous movement model cannot route",
                        def.name
                    );
                }
            }
        }

        // The hierarchy serves one abstraction per distinct mover *shape* the
        // content declares — layers and footprint size together, because a wider
        // mover's map genuinely has fewer ways through it and its regions are
        // coarser. The registry iterates deterministically.
        let mut shapes: Vec<MoverShape> = Vec::new();
        for def in registry.entities() {
            if let Some(location) = def.location
                && def.can_move()
            {
                let shape = MoverShape::new(location.occupation(), location.size());
                if !shapes.contains(&shape) {
                    shapes.push(shape);
                }
            }
        }

        Self::with_hierarchy_shapes(
            data.name(),
            data.projection(),
            data.movement_model(),
            nav_grid,
            start_points,
            &shapes,
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
        Self::with_hierarchy_shapes(
            name,
            projection,
            movement_model,
            nav_grid,
            start_points,
            &[],
        )
    }

    /// Creates a map from loaded content, building one hierarchy abstraction
    /// per given mover shape.
    pub fn with_hierarchy_shapes(
        name: impl Into<String>,
        projection: Projection,
        movement_model: MovementModel,
        nav_grid: NavGrid,
        start_points: Vec<Option<CellPos>>,
        shapes: &[MoverShape],
    ) -> Self {
        let hierarchy = NavHierarchy::build(&nav_grid, DEFAULT_CLUSTER_SIZE, shapes);
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

    // No mutable grid accessor on purpose: every occupancy write goes
    // through a Map method that carries the write's contract — its asserts,
    // its hierarchy bookkeeping, its movement-model discipline.

    /// Writes one cell of the static plane, keeping the hierarchy in sync.
    ///
    /// This is the runtime path for static occupancy that belongs to no
    /// entity footprint — map fixtures and scripted terrain changes; entity
    /// footprints go through [`place_entity`](Self::place_entity) and
    /// [`displace_entity`](Self::displace_entity), which carry their own
    /// bookkeeping through this same write. Valid under both movement
    /// models: unlike the claim plane, the static plane keeps one discipline
    /// everywhere.
    ///
    /// Like every occupancy write, it must flip the cell, and debug builds
    /// assert it: the plane's bits carry no owner, so two writers blocking
    /// the same cell would silently merge and the first to free it would
    /// free it for both — the flip is the only point where that collision
    /// can surface. A caller that has not read the cell checks it first.
    pub fn set_static_occupied(
        &mut self,
        mask: impl Into<LayerMask>,
        pos: CellPos,
        occupied: bool,
    ) {
        let mask = mask.into();
        // Asserted against the static plane alone: this writes only static
        // bits, and a mover's claim over the cell says nothing about them.
        debug_assert!(
            self.nav_grid.is_statically_occupied_by(mask, pos) != occupied,
            "a static write must flip the cell: blocking needs it free, \
             freeing needs it blocked"
        );
        self.nav_grid.set_occupied_by(mask, pos, occupied);
        self.hierarchy.mark_dirty(pos);
    }

    /// Rebuilds the claim plane as the continuous model's once-per-tick
    /// summary: wipes it, stamps each settled footprint, then re-asserts the
    /// reserved ground — claim state with no body standing on it, which the
    /// wipe would otherwise evaporate.
    ///
    /// Panics under the cell model, where claims are law moved cell by cell
    /// and a wipe would destroy what they record.
    pub fn rebuild_claims(
        &mut self,
        footprints: &[(LayerMask, CellSize, CellPos)],
        reservations: &[(LayerMask, Vec<CellPos>)],
    ) {
        match self.movement_model {
            MovementModel::Cell => {
                panic!("the claim plane is never rebuilt under the cell model")
            }
            MovementModel::Continuous => {}
        }
        self.nav_grid.clear_claims();
        for &(mask, size, anchor) in footprints {
            for cell in CellRect::new(anchor, size).cells() {
                self.nav_grid.set_claimed_by(mask, cell, true);
            }
        }
        for (mask, cells) in reservations {
            for &cell in cells {
                self.nav_grid.set_claimed_by(*mask, cell, true);
            }
        }
    }

    /// Whether a footprint of `size` could step from the rect at `from` to the
    /// one at `to`: every cell of the destination the mover does not already
    /// hold must be free.
    ///
    /// The mover's own cells are excluded rather than released and re-tested,
    /// because a step of one cell keeps most of its footprint where it was and
    /// would otherwise read its own claim as a blockage.
    pub fn can_step_footprint(
        &self,
        occupation: LayerMask,
        size: CellSize,
        from: CellPos,
        to: CellPos,
    ) -> bool {
        Self::entered_cells(size, from, to)
            .all(|cell| self.nav_grid.is_passable_by(occupation, cell))
    }

    /// Moves a mover's claim from the footprint at `from` to the one at `to`,
    /// releasing what it leaves and claiming what it enters.
    ///
    /// Only the cells that actually change hands are written, so the overlap a
    /// one-cell step keeps is never released — which is what lets the cell
    /// model's one-claimant-per-cell contract hold through a crossing.
    ///
    /// See also [`take_claim`](Self::take_claim) for briefly lifting a claim
    /// out of a search's way.
    pub fn step_claim(
        &mut self,
        occupation: LayerMask,
        size: CellSize,
        from: CellPos,
        to: CellPos,
    ) {
        for cell in Self::entered_cells(size, to, from) {
            debug_assert!(
                self.nav_grid.is_claimed_by(occupation, cell),
                "a crossing must release cells the mover holds"
            );
            self.nav_grid.set_claimed_by(occupation, cell, false);
        }
        for cell in Self::entered_cells(size, from, to) {
            debug_assert!(
                !self.nav_grid.is_claimed_by(occupation, cell),
                "a crossing must claim free cells"
            );
            self.nav_grid.set_claimed_by(occupation, cell, true);
        }
    }

    /// Releases the claim on every cell of the `size` footprint at `origin`
    /// that is held on `mask`, returning exactly the cells released so
    /// [`restore_claim`](Self::restore_claim) can put them back.
    ///
    /// This is how a search stops reading the searcher's own claim as an
    /// obstacle: a mover's footprint claims real cells, and a claim-honoring
    /// route out of its own resting spot must not be walled in by itself. The
    /// take-and-restore pair brackets a single query — nothing else may run in
    /// between, or it would see claims missing that no one released for it.
    ///
    /// Under the cell model the claim plane is law and the query runs for a
    /// settled claimant, so the whole rect must be claimed — a gap means a
    /// corrupted plane, and debug builds assert against it. Under the
    /// continuous model the rect is filtered instead: the plane is a
    /// once-per-tick summary rebuilt from where bodies last settled, and a
    /// body that moved since legitimately stands off its own claim. Recording
    /// exactly what was flipped keeps the bracket self-inverse either way —
    /// restoring the whole rect would mint claims that never existed.
    pub fn take_claim(&mut self, mask: LayerMask, origin: CellPos, size: CellSize) -> Vec<CellPos> {
        let mut held = Vec::new();
        for cell in CellRect::new(origin, size).cells() {
            match self.movement_model {
                MovementModel::Cell => {
                    debug_assert!(
                        self.nav_grid.is_claimed_by(mask, cell),
                        "a settled claimant's footprint must be fully claimed \
                         under the cell model"
                    );
                }
                MovementModel::Continuous => {
                    if !self.nav_grid.is_claimed_by(mask, cell) {
                        continue;
                    }
                }
            }
            self.nav_grid.set_claimed_by(mask, cell, false);
            held.push(cell);
        }
        held
    }

    /// Puts back the claims a [`take_claim`](Self::take_claim) released.
    ///
    /// The bracket is exclusive, so the cells must still be free exactly as
    /// the take left them; one found claimed means something wrote into the
    /// bracket.
    pub fn restore_claim(&mut self, mask: LayerMask, cells: &[CellPos]) {
        for &cell in cells {
            debug_assert!(
                !self.nav_grid.is_claimed_by(mask, cell),
                "restoring a taken claim must find its cells free"
            );
            self.nav_grid.set_claimed_by(mask, cell, true);
        }
    }

    /// Claims every cell of the `size` footprint at `origin` not already
    /// claimed on `mask`, returning exactly the cells claimed so
    /// [`release_claim`](Self::release_claim) can let them go.
    ///
    /// This is how ground is secured ahead of standing on it: the claims read
    /// as occupied to placement, spawning, and claim-honoring movement, while
    /// cells someone else already holds stay theirs — the caller is expected
    /// to have verified the footprint is free first.
    ///
    /// The rect is filtered rather than asserted free, because the reserving
    /// entity's own standing claim may already hold destination cells on
    /// shared layers; those stay under the standing claim, and releasing only
    /// what this call flipped is what lets a cancelled reservation leave the
    /// standing claim intact.
    pub fn reserve_claim(
        &mut self,
        mask: LayerMask,
        origin: CellPos,
        size: CellSize,
    ) -> Vec<CellPos> {
        let mut claimed = Vec::new();
        for cell in CellRect::new(origin, size).cells() {
            if !self.nav_grid.is_claimed_by(mask, cell) {
                self.nav_grid.set_claimed_by(mask, cell, true);
                claimed.push(cell);
            }
        }
        claimed
    }

    /// Lets go of the claims a [`reserve_claim`](Self::reserve_claim) took.
    ///
    /// Reserved cells read as occupied until this call, so they must all
    /// still be claimed — under the cell model nothing may write over a
    /// reservation, and under the continuous model every rebuild re-asserts
    /// it; one found free means the reservation was clobbered.
    pub fn release_claim(&mut self, mask: LayerMask, cells: &[CellPos]) {
        for &cell in cells {
            debug_assert!(
                self.nav_grid.is_claimed_by(mask, cell),
                "releasing a reservation must find its cells claimed"
            );
            self.nav_grid.set_claimed_by(mask, cell, false);
        }
    }

    /// The cells of the `size` footprint at `to` that the one at `from` does not
    /// already cover, in row-major order.
    fn entered_cells(size: CellSize, from: CellPos, to: CellPos) -> impl Iterator<Item = CellPos> {
        (0..size.height).flat_map(move |dy| {
            (0..size.width).filter_map(move |dx| {
                let cell = CellPos::new(to.x + dx, to.y + dy);
                let covered = cell.x >= from.x
                    && cell.x < from.x + size.width
                    && cell.y >= from.y
                    && cell.y < from.y + size.height;
                (!covered).then_some(cell)
            })
        })
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
                    // Solid footprints exclude each other and placement is
                    // validated against terrain, so the flip the static
                    // write asserts holds for footprints under both models.
                    OccupancyClass::Static => {
                        self.set_static_occupied(location_def.occupation(), cell, occupied);
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
