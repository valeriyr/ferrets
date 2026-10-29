//! The live map grid of the active game.

use std::cmp::Ordering;

use bevy_ecs::prelude::*;
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_nav_grid::{
    layer_id::LayerId,
    layer_level::LayerLevel,
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    slope::{Direction, Slope},
    terrain_id::TerrainId,
};
use ferrets_pathfinder::{
    hierarchy::{DEFAULT_CLUSTER_SIZE, NavHierarchy},
    search,
};

use ferrets_math::FixedU64;
use ferrets_physics::body;

use crate::{
    components::location::LocationComponent,
    entity_def,
    map_data::{MapData, MapSlot, Placement},
    movement_model::MovementModel,
    occupancy::{GridMark, OccupancyClass},
    session::player_id::PlayerId,
};
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, registry::ContentRegistry,
};

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
    /// layer of the registered vocabulary, each cell of the terrain the map
    /// gives it, and each layer spanning levels of the relief the map gives
    /// it; entity occupancy composes on top.
    ///
    /// Panics if the terrain palette references an unregistered terrain, a cell
    /// indexes past the palette, a per-cell list does not cover the grid, the
    /// map gives a relief to a layer at one level or to no registered layer,
    /// or none to a layer spanning levels, a cell's level lies outside its
    /// layer's, a slope breaks its band's rules, a border between levels lies
    /// in no band, or a placement starts a pool its type cannot start there.
    pub fn from_data(data: &MapData, registry: &ContentRegistry) -> Self {
        assert_eq!(
            data.terrain_cells().len(),
            data.width() as usize * data.height() as usize,
            "map '{}' terrain must cover every cell",
            data.name()
        );
        let palette: Vec<TerrainId> = data
            .terrains()
            .iter()
            .map(|name| {
                registry.terrain(name).unwrap_or_else(|| {
                    panic!("map '{}' uses unregistered terrain '{name}'", data.name())
                })
            })
            .collect();
        let cells: Vec<TerrainId> = data
            .terrain_cells()
            .iter()
            .enumerate()
            .map(|(i, &terrain)| {
                *palette.get(terrain as usize).unwrap_or_else(|| {
                    panic!(
                        "map '{}' cell {i} indexes past the terrain palette",
                        data.name()
                    )
                })
            })
            .collect();
        let cell_count = data.width() as usize * data.height() as usize;
        let width = data.width();
        let mut nav_grid = NavGrid::new(width, data.height(), |pos| {
            cells[(pos.y * width + pos.x) as usize]
        });
        for (name, _) in data.reliefs() {
            assert!(
                registry.layer(name).is_some(),
                "map '{}' gives a relief to the unregistered layer '{name}'",
                data.name()
            );
        }
        for (name, layer) in registry.layers() {
            match registry.layer_level(layer) {
                LayerLevel::At(level) => {
                    assert!(
                        data.relief(name).is_none(),
                        "map '{}' gives a relief to layer '{name}', which is at the one level {level}",
                        data.name()
                    );
                    nav_grid.add_flat_layer(layer, level);
                }
                levels @ LayerLevel::Range { from, to } => {
                    let relief = data.relief(name).unwrap_or_else(|| {
                        panic!(
                            "map '{}' gives no relief to layer '{name}', which spans levels {from}..={to}",
                            data.name()
                        )
                    });
                    assert_eq!(
                        relief.len(),
                        cell_count,
                        "map '{}' relief of layer '{name}' must cover every cell",
                        data.name()
                    );
                    for (i, cell) in relief.iter().enumerate() {
                        assert!(
                            levels.holds(cell.level),
                            "map '{}' cell {i} of layer '{name}' is at level {}, outside {from}..={to}",
                            data.name(),
                            cell.level
                        );
                    }
                    nav_grid.add_ranged_layer(layer, from, to, |pos| {
                        relief[(pos.y * width + pos.x) as usize]
                    });
                    let whose = format!("map '{}' layer '{name}'", data.name());
                    for cell in
                        CellRect::new(CellPos::new(0, 0), CellSize::new(width, data.height()))
                            .cells()
                    {
                        check_slope(&nav_grid, layer, &whose, cell);
                        for edge in [Direction::East, Direction::South] {
                            check_border(&nav_grid, layer, &whose, cell, edge);
                        }
                    }
                }
            }
        }

        // What a death leaves is laid by the death that leaves it; a map that
        // opens with bodies already on it would have nobody they were left by.
        for placement in data.placements() {
            let laid = registry
                .entity(&placement.type_name)
                .is_some_and(EntityTypeDef::is_remains);
            assert!(
                !laid,
                "placement '{}' names remains, which only a death may leave",
                placement.type_name
            );
            check_placement_pools(placement, registry);
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
                    let radius = def.base_stat(EntityStatId::RADIUS);
                    assert!(
                        radius.is_some_and(|radius| radius > FixedU64::ZERO),
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
                        radius.is_some_and(|radius| radius <= bound),
                        "entity type '{}' authors a radius beyond half its \
                         footprint's narrow side ({bound}), which the \
                         continuous movement model cannot route",
                        def.name
                    );
                    // Contact splits a penetration in proportion to what the two
                    // bodies weigh, so every mover states what it weighs against
                    // what it meets. Only stating it is required, not stating
                    // anything in particular: nothing weighs against zero, which
                    // is how content spells a body that yields to everything and
                    // shoves nothing.
                    assert!(
                        def.base_stat(EntityStatId::WEIGHT).is_some(),
                        "entity type '{}' moves but defines no weight, \
                         which the continuous movement model requires",
                        def.name
                    );
                }
            }
        }

        let mut map = Self::new(
            data.name(),
            data.projection(),
            data.movement_model(),
            nav_grid,
            start_points,
        );
        map.serve_movers(registry);
        map
    }

    /// Creates a map over `nav_grid` whose hierarchy serves no mover yet.
    pub fn new(
        name: impl Into<String>,
        projection: Projection,
        movement_model: MovementModel,
        nav_grid: NavGrid,
        start_points: Vec<Option<CellPos>>,
    ) -> Self {
        let hierarchy = NavHierarchy::build(&nav_grid, DEFAULT_CLUSTER_SIZE, &[]);
        Self {
            name: name.into(),
            projection,
            movement_model,
            nav_grid,
            hierarchy,
            start_points,
        }
    }

    /// Rebuilds the hierarchy over the grid as it stands, one abstraction per
    /// mover shape `registry` declares.
    pub fn serve_movers(&mut self, registry: &ContentRegistry) {
        self.hierarchy = NavHierarchy::build(
            &self.nav_grid,
            DEFAULT_CLUSTER_SIZE,
            &Self::mover_shapes(registry),
        );
    }

    /// Returns a reference to the hierarchical view of the grid's static
    /// plane.
    pub fn hierarchy(&self) -> &NavHierarchy {
        &self.hierarchy
    }

    /// Panics naming the first mover type `registry` declares whose shape the
    /// hierarchy does not serve.
    pub fn assert_serves_movers(&self, registry: &ContentRegistry) {
        for def in registry.entities() {
            if let Some(shape) = Self::planned_shape(registry, def) {
                assert!(
                    self.hierarchy.serves(shape),
                    "map '{}' serves no hierarchy for mover '{}' (terrains {}, layers {} at {}x{}, cliff crossing {:?})",
                    self.name,
                    def.name,
                    shape.terrains,
                    shape.layers,
                    shape.size.width,
                    shape.size.height,
                    shape.cliff_crossing
                );
            }
        }
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

    /// Whether `pos` lies on the map.
    #[inline]
    pub fn contains(&self, pos: CellPos) -> bool {
        pos.x < self.width() && pos.y < self.height()
    }

    /// Returns a reference to the navigation grid.
    pub fn nav_grid(&self) -> &NavGrid {
        &self.nav_grid
    }

    /// Returns `true` if no standing footprint holds a cell of a footprint of
    /// `to` at `placed` on its layers but for what `lifted` marks there,
    /// whatever the ground under it and whoever's claims lie on it.
    pub fn is_statically_free_over_lifted(
        &self,
        placed: &LocationComponent,
        to: MoverShape,
        lifted: &[GridMark],
    ) -> bool {
        let anchor = body::anchor(placed.position);
        CellRect::new(anchor, to.size).cells().all(|cell| {
            let static_lifted = lifted
                .iter()
                .filter(|mark| mark.footprint.contains(cell))
                .fold(LayerMask::EMPTY, |layers, mark| match mark.class {
                    OccupancyClass::Static => layers | mark.layers,
                    OccupancyClass::Claim => layers,
                });
            let beyond = to.layers & !static_lifted;
            beyond == LayerMask::EMPTY || !self.nav_grid.is_statically_occupied_by(beyond, cell)
        })
    }

    /// Returns `true` if every cell in a footprint of `to` at `placed` is free
    /// on its layers but for what `lifted` marks there, whatever the ground
    /// under it.
    pub fn is_free_over_lifted(
        &self,
        placed: &LocationComponent,
        to: MoverShape,
        lifted: &[GridMark],
    ) -> bool {
        let layers = to.layers;
        let anchor = body::anchor(placed.position);
        CellRect::new(anchor, to.size).cells().all(|cell| {
            if !self.nav_grid.is_occupied_by(layers, cell) {
                return true;
            }
            // The layers lifted bodies mark on this cell, plane by plane.
            let (mut static_lifted, mut claim_lifted) = (LayerMask::EMPTY, LayerMask::EMPTY);
            for mark in lifted {
                if !mark.footprint.contains(cell) {
                    continue;
                }
                match mark.class {
                    OccupancyClass::Static => static_lifted |= mark.layers,
                    OccupancyClass::Claim => claim_lifted |= mark.layers,
                }
            }
            if static_lifted == LayerMask::EMPTY && claim_lifted == LayerMask::EMPTY {
                return false;
            }
            // Whatever else is here is on a layer or plane the lifted
            // bodies do not mark.
            let beyond_static = layers & !static_lifted;
            let beyond_claim = layers & !claim_lifted;
            (beyond_static == LayerMask::EMPTY
                || !self.nav_grid.is_statically_occupied_by(beyond_static, cell))
                && (beyond_claim == LayerMask::EMPTY
                    || !self.nav_grid.is_claimed_by(beyond_claim, cell))
        })
    }

    /// Finds the first anchor near the rectangle at `origin` that `accept`
    /// takes for a `footprint`-sized body, within `radius` rings of it: ring by
    /// ring outward, each cell once, within a ring the ground closest to
    /// `toward` first.
    pub fn find_placement_near_toward_where(
        &self,
        origin: CellPos,
        size: CellSize,
        footprint: CellSize,
        radius: u32,
        toward: CellRect,
        accept: impl FnMut(CellPos) -> bool,
    ) -> Option<CellPos> {
        search::find_placement_near_toward_where(
            &self.nav_grid,
            CellRect::new(origin, size),
            footprint,
            radius,
            toward,
            accept,
        )
    }

    /// Marks every cell `mark` covers as occupied on its layers, on the plane
    /// its class selects.
    pub fn place(&mut self, mark: GridMark) {
        self.set_mark(mark, true);
    }

    /// Clears every cell `mark` covers on its layers, on the plane its class
    /// selects.
    pub fn displace(&mut self, mark: GridMark) {
        self.set_mark(mark, false);
    }

    /// [`place`](Self::place)s `mark` when there is one; a body whose
    /// solidity claims no cells has none.
    pub fn place_if_some(&mut self, mark: Option<GridMark>) {
        if let Some(mark) = mark {
            self.place(mark);
        }
    }

    /// [`displace`](Self::displace)s `mark` when there is one; a body whose
    /// solidity claims no cells has none.
    pub fn displace_if_some(&mut self, mark: Option<GridMark>) {
        if let Some(mark) = mark {
            self.displace(mark);
        }
    }

    // No mutable grid accessor on purpose: every occupancy write goes
    // through a Map method that carries the write's contract — its asserts,
    // its hierarchy bookkeeping, its movement-model discipline.

    /// Writes one cell of the static plane, keeping the hierarchy in sync.
    ///
    /// This is the runtime path for static occupancy that belongs to no
    /// entity footprint — map fixtures and scripted blockers; entity
    /// footprints go through [`place`](Self::place) and
    /// [`displace`](Self::displace), which carry their own
    /// bookkeeping through this same write. Valid under both movement
    /// models: unlike the claim plane, the static plane keeps one discipline
    /// everywhere.
    ///
    /// Like every occupancy write, it must flip the cell's static reading, and
    /// debug builds assert it: the plane's bits carry no owner, so two writers
    /// blocking the same cell would silently merge and the first to free it
    /// would free it for both. A caller that has not read the cell checks it
    /// first.
    ///
    /// Panics for a cell off the grid or an empty mask.
    pub fn set_static_occupied(
        &mut self,
        layers: impl Into<LayerMask>,
        pos: CellPos,
        occupied: bool,
    ) {
        let layers = layers.into();
        assert!(
            layers != LayerMask::EMPTY,
            "a footprint occupies some layer"
        );
        assert!(self.contains(pos), "a footprint stands on the grid");
        // Asserted against the static plane alone: this writes only static
        // bits, and a mover's claim over the cell says nothing about them.
        debug_assert!(
            self.nav_grid.is_statically_occupied_by(layers, pos) != occupied,
            "a static write must flip the cell: blocking needs it free, \
             freeing needs it blocked"
        );
        self.nav_grid.set_occupied_by(layers, pos, occupied);
        self.hierarchy.mark_dirty(pos, layers);
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
        for &(layers, size, anchor) in footprints {
            for cell in CellRect::new(anchor, size).cells() {
                self.nav_grid.set_claimed_by(layers, cell, true);
            }
        }
        for (layers, cells) in reservations {
            for &cell in cells {
                self.nav_grid.set_claimed_by(*layers, cell, true);
            }
        }
    }

    /// Whether a footprint of `shape` could step from the rect at `from` to
    /// the neighboring one at `to`: every cell of the destination the mover
    /// does not already hold must be ground it may stand on (see
    /// [`NavGrid::cell_fits`]) and free, the step must cross only edges its
    /// cliff crossing allows, and the destination footprint must hold
    /// together (see [`NavGrid::holds_together`]).
    ///
    /// The mover's own cells are excluded rather than released and re-tested,
    /// because a step of one cell keeps most of its footprint where it was and
    /// would otherwise read its own claim as a blockage.
    pub fn can_step_footprint(&self, shape: MoverShape, from: CellPos, to: CellPos) -> bool {
        Self::entered_cells(shape.size, from, to).all(|cell| self.nav_grid.cell_fits(cell, shape))
            && self.nav_grid.can_step(from, to, shape)
            && self.nav_grid.holds_together(to, shape)
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
    pub fn step_claim(&mut self, layers: LayerMask, size: CellSize, from: CellPos, to: CellPos) {
        for cell in Self::entered_cells(size, to, from) {
            debug_assert!(
                self.nav_grid.is_claimed_by(layers, cell),
                "a crossing must release cells the mover holds"
            );
            self.nav_grid.set_claimed_by(layers, cell, false);
        }
        for cell in Self::entered_cells(size, from, to) {
            debug_assert!(
                !self.nav_grid.is_claimed_by(layers, cell),
                "a crossing must claim free cells"
            );
            self.nav_grid.set_claimed_by(layers, cell, true);
        }
    }

    /// Releases the claim on every cell of the `size` footprint at `origin`
    /// that is held on `layers`, returning exactly the cells released so
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
    pub fn take_claim(
        &mut self,
        layers: LayerMask,
        origin: CellPos,
        size: CellSize,
    ) -> Vec<CellPos> {
        let mut held = Vec::new();
        for cell in CellRect::new(origin, size).cells() {
            match self.movement_model {
                MovementModel::Cell => {
                    debug_assert!(
                        self.nav_grid.is_claimed_by(layers, cell),
                        "a settled claimant's footprint must be fully claimed \
                         under the cell model"
                    );
                }
                MovementModel::Continuous => {
                    if !self.nav_grid.is_claimed_by(layers, cell) {
                        continue;
                    }
                }
            }
            self.nav_grid.set_claimed_by(layers, cell, false);
            held.push(cell);
        }
        held
    }

    /// Puts back the claims a [`take_claim`](Self::take_claim) released.
    ///
    /// The bracket is exclusive, so the cells must still be free exactly as
    /// the take left them; one found claimed means something wrote into the
    /// bracket.
    pub fn restore_claim(&mut self, layers: LayerMask, cells: &[CellPos]) {
        for &cell in cells {
            debug_assert!(
                !self.nav_grid.is_claimed_by(layers, cell),
                "restoring a taken claim must find its cells free"
            );
            self.nav_grid.set_claimed_by(layers, cell, true);
        }
    }

    /// Claims every cell of the `size` footprint at `origin` not already
    /// claimed on `layers`, returning exactly the cells claimed so
    /// [`release_claim`](Self::release_claim) can let them go.
    ///
    /// This is how ground is secured ahead of standing on it: the claims read
    /// as occupied to placement, spawning, and claim-honoring movement. Cells
    /// `standing` — what the reserving entity marks where it stands now —
    /// already claims stay under that claim and are not returned.
    ///
    /// Expects every other cell unclaimed; panics in debug builds otherwise.
    pub fn reserve_claim(
        &mut self,
        layers: LayerMask,
        origin: CellPos,
        size: CellSize,
        standing: Option<GridMark>,
    ) -> Vec<CellPos> {
        let mut claimed = Vec::new();
        for cell in CellRect::new(origin, size).cells() {
            if !self.nav_grid.is_claimed_by(layers, cell) {
                self.nav_grid.set_claimed_by(layers, cell, true);
                claimed.push(cell);
                continue;
            }
            debug_assert!(
                standing.is_some_and(|mark| match mark.class {
                    OccupancyClass::Claim => {
                        mark.footprint.contains(cell) && mark.layers & layers != LayerMask::EMPTY
                    }
                    OccupancyClass::Static => false,
                }),
                "a reservation finds a cell claimed only under the reserver's own standing claim"
            );
        }
        claimed
    }

    /// Lets go of the claims a [`reserve_claim`](Self::reserve_claim) took.
    ///
    /// Reserved cells read as occupied until this call, so they must all
    /// still be claimed — under the cell model nothing may write over a
    /// reservation, and under the continuous model every rebuild re-asserts
    /// it; one found free means the reservation was clobbered.
    pub fn release_claim(&mut self, layers: LayerMask, cells: &[CellPos]) {
        for &cell in cells {
            debug_assert!(
                self.nav_grid.is_claimed_by(layers, cell),
                "releasing a reservation must find its cells claimed"
            );
            self.nav_grid.set_claimed_by(layers, cell, false);
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

    /// Marks or clears every cell `mark` covers, as `occupied` says.
    ///
    /// Static marks dirty the hierarchy; claims never do.
    fn set_mark(&mut self, mark: GridMark, occupied: bool) {
        let layers = mark.layers;
        for cell in mark.footprint.cells() {
            match mark.class {
                // Solid footprints exclude each other and placement is
                // validated against terrain, so the flip the static
                // write asserts holds for footprints under both models.
                OccupancyClass::Static => {
                    self.set_static_occupied(layers, cell, occupied);
                }
                OccupancyClass::Claim => {
                    // One claimant per cell per layer is the cell
                    // model's contract; the continuous model rebuilds
                    // the plane from bodies each tick and legally
                    // collapses shared cells into one bit — and its
                    // clears belong to that rebuild alone: a footprint
                    // release could wipe a neighbor's claim on a shared
                    // cell mid-tick.
                    match self.movement_model {
                        MovementModel::Cell => {
                            debug_assert!(
                                self.nav_grid.is_claimed_by(layers, cell) != occupied,
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
                    self.nav_grid.set_claimed_by(layers, cell, occupied);
                }
            }
        }
    }

    /// Every distinct mover shape the content declares, in name order:
    /// the abstractions a map must serve for each of its movers to plan.
    fn mover_shapes(registry: &ContentRegistry) -> Vec<MoverShape> {
        let mut shapes: Vec<MoverShape> = Vec::new();
        for shape in registry
            .entities()
            .filter_map(|def| Self::planned_shape(registry, def))
        {
            if !shapes.contains(&shape) {
                shapes.push(shape);
            }
        }
        shapes
    }

    /// The shape a type plans with, or `None` for a type that does not move.
    fn planned_shape(registry: &ContentRegistry, def: &EntityTypeDef) -> Option<MoverShape> {
        def.location
            .filter(|_| def.can_move())
            .map(|location| entity_def::mover_shape(registry, &location))
    }
}

/// Checks the slope at `cell` on `layer`, if there is one: a ramp rises a
/// cardinal way, and the band it lies in — the run of slopes of its kind,
/// leaning its way, through it along the way it rises — never steps down,
/// changes level exactly once, and a ramp by one level. `whose` names the map
/// and layer in a panic.
fn check_slope(grid: &NavGrid, layer: LayerId, whose: &str, cell: CellPos) {
    let Some(slope) = grid.slope(layer, cell) else {
        return;
    };
    let rises = slope.rises();
    match slope {
        Slope::Ramp(_) => assert!(
            rises.is_cardinal(),
            "{whose} ramp at ({}, {}) rises {rises:?}, which is not cardinal",
            cell.x,
            cell.y
        ),
        Slope::Cliff(_) => {}
    }
    let in_band = |at: Option<CellPos>| {
        at.filter(|&at| {
            grid.slope(layer, at)
                .is_some_and(|other| other.is_same_kind(slope) && leans_with(other.rises(), rises))
        })
    };
    let mut start = cell;
    while let Some(behind) = in_band(neighbor(grid, start, rises.opposite())) {
        start = behind;
    }
    let (mut at, mut changes) = (start, 0);
    while let Some(next) = in_band(neighbor(grid, at, rises)) {
        let (Some(level), Some(next_level)) =
            (grid.cell_level(layer, at), grid.cell_level(layer, next))
        else {
            unreachable!("cells on the grid have a level");
        };
        match (slope, next_level.cmp(&level)) {
            (Slope::Ramp(_) | Slope::Cliff(_), Ordering::Equal) => {}
            (Slope::Ramp(_), Ordering::Greater) if next_level == level + 1 => changes += 1,
            (Slope::Cliff(_), Ordering::Greater) => changes += 1,
            (Slope::Ramp(_), Ordering::Greater) => panic!(
                "{whose} ramp band through ({}, {}) climbs more than one level from ({}, {})",
                cell.x, cell.y, at.x, at.y
            ),
            (Slope::Ramp(_) | Slope::Cliff(_), Ordering::Less) => panic!(
                "{whose} {slope:?} band through ({}, {}) steps down from ({}, {})",
                cell.x, cell.y, at.x, at.y
            ),
        }
        at = next;
    }
    assert!(
        changes == 1,
        "{whose} {slope:?} band through ({}, {}) changes level {changes} times, not once",
        cell.x,
        cell.y
    );
}

/// Checks the edge on `layer` from `cell` to its neighbor `edge`'s way, if
/// the two differ in level: the lower a slope rising across the edge, the
/// higher a slope of the same kind, and a ramp one level apart. `whose` names
/// the map and layer in a panic.
fn check_border(grid: &NavGrid, layer: LayerId, whose: &str, cell: CellPos, edge: Direction) {
    let Some(next) = neighbor(grid, cell, edge) else {
        return;
    };
    let (Some(relief), Some(next_relief)) = (grid.relief(layer, cell), grid.relief(layer, next))
    else {
        unreachable!("cells on the grid have a relief");
    };
    let ((lower, lower_relief), (upper, upper_relief), rising) =
        match relief.level.cmp(&next_relief.level) {
            Ordering::Equal => return,
            Ordering::Less => ((cell, relief), (next, next_relief), edge),
            Ordering::Greater => ((next, next_relief), (cell, relief), edge.opposite()),
        };
    let in_band = match (lower_relief.slope, upper_relief.slope) {
        (Some(low), Some(high)) => low.rises().crosses(rising) && low.is_same_kind(high),
        (Some(_), None) | (None, Some(_)) | (None, None) => false,
    };
    assert!(
        in_band,
        "{whose} border from ({}, {}) up to ({}, {}) lies in no ramp or cliff",
        lower.x, lower.y, upper.x, upper.y
    );
    let rise = upper_relief.level - lower_relief.level;
    match lower_relief.slope {
        Some(Slope::Ramp(_)) => assert!(
            rise == 1,
            "{whose} ramp from ({}, {}) up to ({}, {}) climbs {rise} levels, not one",
            lower.x,
            lower.y,
            upper.x,
            upper.y
        ),
        Some(Slope::Cliff(_)) => {}
        None => unreachable!("a border in a band has a slope below"),
    }
}

/// Whether `direction` leans at least partly the way `other` does.
fn leans_with(direction: Direction, other: Direction) -> bool {
    let ((dx, dy), (ox, oy)) = (direction.step(), other.step());
    dx * ox + dy * oy > 0
}

/// The cell `direction`'s way from `cell`, or `None` off the grid.
fn neighbor(grid: &NavGrid, cell: CellPos, direction: Direction) -> Option<CellPos> {
    let (dx, dy) = direction.step();
    cell.x
        .checked_add_signed(dx)
        .zip(cell.y.checked_add_signed(dy))
        .map(|(x, y)| CellPos::new(x, y))
        .filter(|&next| next.x < grid.width() && next.y < grid.height())
}

/// Checks each pool `placement` starts: a registered pool, named once, and
/// for a registered type one it declares, at an initial the pool can start
/// at.
fn check_placement_pools(placement: &Placement, registry: &ContentRegistry) {
    for (index, (name, initial)) in placement.pools.iter().enumerate() {
        let pool = registry.pool(name).unwrap_or_else(|| {
            panic!(
                "placement '{}' names the unregistered pool '{name}'",
                placement.type_name
            )
        });
        assert!(
            placement.pools[..index]
                .iter()
                .all(|(earlier, _)| earlier != name),
            "placement '{}' names the {name} pool twice",
            placement.type_name
        );
        // A placement of an unregistered type is skipped when the map is
        // built, so only its names are checked.
        let Some(def) = registry.entity(&placement.type_name) else {
            continue;
        };
        let declaration = def.base_stats.pool(pool).unwrap_or_else(|| {
            panic!(
                "placement '{}' names the {name} pool, which its type does not declare",
                placement.type_name
            )
        });
        declaration.validate_initial(
            *initial,
            &format!("placement '{}' starts the {name} pool", placement.type_name),
        );
    }
}
