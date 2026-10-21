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
    search::{self, Footing},
    terrain_id::TerrainId,
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
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::Solidity,
    registry::ContentRegistry,
};

/// How far out from a rectangle a placement search looks before giving up.
const MAX_RADIUS: u32 = 8;

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
    /// layer of the registered vocabulary, and each cell of the terrain the
    /// map gives it; entity occupancy composes on top.
    ///
    /// Panics if the terrain palette references an unregistered terrain, a cell
    /// indexes past the palette, the cell count does not match the grid, or a
    /// placement starts a pool its type cannot start there.
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
        let width = data.width();
        let mut nav_grid = NavGrid::new(width, data.height(), |pos| {
            cells[(pos.y * width + pos.x) as usize]
        });
        for (_, layer) in registry.layers() {
            nav_grid.add_layer(layer);
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
                    "map '{}' serves no hierarchy for mover '{}' (terrains {}, layers {} at {}x{})",
                    self.name,
                    def.name,
                    shape.terrains,
                    shape.layers,
                    shape.size.width,
                    shape.size.height
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

    /// Returns `true` if every cell in a footprint of `to` at `placed` is
    /// on its terrains and free but for what `lifted` marks there.
    pub fn can_place_entity_over_lifted(
        &self,
        placed: &LocationComponent,
        to: MoverShape,
        lifted: &[GridMark],
    ) -> bool {
        let layers = to.layers;
        CellRect::new(body::anchor(placed.position), to.size)
            .cells()
            .all(|cell| {
                if !self.nav_grid.is_on_terrains(to.terrains, cell) {
                    return false;
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
                    return !self.nav_grid.is_occupied_by(layers, cell);
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

    /// Finds a free position for a body of `shape` and `solidity`, scanning
    /// outward from the rectangle of cells at `origin` with the given `size`.
    ///
    /// Cells are scanned ring by ring in row-major order, so the result is
    /// deterministic. Returns `None` when nothing is free within the search radius.
    pub fn find_placement_near(
        &self,
        origin: CellPos,
        size: CellSize,
        shape: MoverShape,
        solidity: Solidity,
    ) -> Option<CellPos> {
        search::find_placement_near(
            &self.nav_grid,
            shape,
            CellRect::new(origin, size),
            MAX_RADIUS,
            footing(solidity),
        )
    }

    /// Like [`Map::find_placement_near`], within each ring taking the free
    /// position closest to the rectangle `toward` first.
    pub fn find_placement_near_toward(
        &self,
        origin: CellPos,
        size: CellSize,
        shape: MoverShape,
        solidity: Solidity,
        toward: CellRect,
    ) -> Option<CellPos> {
        search::find_placement_near_toward(
            &self.nav_grid,
            shape,
            CellRect::new(origin, size),
            MAX_RADIUS,
            footing(solidity),
            toward,
        )
    }

    /// Finds the first anchor near the rectangle at `origin` that `accept`
    /// takes for a `footprint`-sized body, within the placement search radius,
    /// in the order [`Map::find_placement_near_toward`] tries them.
    pub fn find_placement_near_toward_where(
        &self,
        origin: CellPos,
        size: CellSize,
        footprint: CellSize,
        toward: CellRect,
        accept: impl FnMut(CellPos) -> bool,
    ) -> Option<CellPos> {
        search::find_placement_near_toward_where(
            &self.nav_grid,
            CellRect::new(origin, size),
            footprint,
            MAX_RADIUS,
            toward,
            accept,
        )
    }

    /// Finds at most `max_amount` free placements for a body of `shape` and
    /// `solidity` near the rectangle at `origin`, none overlapping another.
    pub fn find_placements_near(
        &self,
        origin: CellPos,
        size: CellSize,
        shape: MoverShape,
        solidity: Solidity,
        max_amount: usize,
    ) -> Vec<CellPos> {
        search::find_placements_near(
            &self.nav_grid,
            shape,
            CellRect::new(origin, size),
            MAX_RADIUS,
            max_amount,
            footing(solidity),
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
    pub fn set_static_occupied(
        &mut self,
        layers: impl Into<LayerMask>,
        pos: CellPos,
        occupied: bool,
    ) {
        let layers = layers.into();
        // Asserted against the static plane alone: this writes only static
        // bits, and a mover's claim over the cell says nothing about them.
        debug_assert!(
            self.nav_grid.is_statically_occupied_by(layers, pos) != occupied,
            "a static write must flip the cell: blocking needs it free, \
             freeing needs it blocked"
        );
        self.nav_grid.set_occupied_by(layers, pos, occupied);
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
    /// the one at `to`: every cell of the destination the mover does not
    /// already hold must be on its terrains and free.
    ///
    /// The mover's own cells are excluded rather than released and re-tested,
    /// because a step of one cell keeps most of its footprint where it was and
    /// would otherwise read its own claim as a blockage.
    pub fn can_step_footprint(&self, shape: MoverShape, from: CellPos, to: CellPos) -> bool {
        Self::entered_cells(shape.size, from, to)
            .all(|cell| self.nav_grid.cell_fits(cell, shape.terrains, shape.layers))
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
    /// as occupied to placement, spawning, and claim-honoring movement, while
    /// cells someone else already holds stay theirs — the caller is expected
    /// to have verified the footprint is free first.
    ///
    /// The rect is filtered rather than asserted free, because the reserving
    /// entity's own standing claim may already hold destination cells on
    /// shared layers; those stay under the standing claim, and releasing only
    /// what this call flipped is what lets a canceled reservation leave the
    /// standing claim intact.
    pub fn reserve_claim(
        &mut self,
        layers: LayerMask,
        origin: CellPos,
        size: CellSize,
    ) -> Vec<CellPos> {
        let mut claimed = Vec::new();
        for cell in CellRect::new(origin, size).cells() {
            if !self.nav_grid.is_claimed_by(layers, cell) {
                self.nav_grid.set_claimed_by(layers, cell, true);
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

/// The ground a footprint of `solidity` needs: one that claims its cells needs
/// them free, one that shares them needs only terrain that allows it.
fn footing(solidity: Solidity) -> Footing {
    match solidity {
        Solidity::Solid => Footing::Free,
        Solidity::Underfoot | Solidity::Passable => Footing::Shared,
    }
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
