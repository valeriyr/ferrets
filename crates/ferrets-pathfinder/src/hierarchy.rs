//! Hierarchical view of the navigation grid: clusters, entrance transitions,
//! and connectivity regions.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Mutex,
};

use crate::{
    astar,
    mover_profile::{Blockers, MoverProfile},
    mover_shape::MoverShape,
    nav_grid::NavGrid,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};

/// Cells per cluster side used by the live game grid.
pub const DEFAULT_CLUSTER_SIZE: u32 = 16;

/// Entrances at least this many cells wide get a transition at each end
/// instead of a single one in the middle.
const WIDE_ENTRANCE: u32 = 6;

/// Region value marking an impassable cell.
const NO_REGION: u32 = u32::MAX;

/// A cluster's coordinates in the cluster grid laid over the navigation grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterPos {
    pub x: u32,
    pub y: u32,
}

impl ClusterPos {
    /// Whether `other` is this cluster or one of its eight neighbors.
    /// Cluster adjacency is lattice structure, identical under every
    /// projection — not a map-metric distance.
    pub fn touches(self, other: Self) -> bool {
        self.x.abs_diff(other.x) <= 1 && self.y.abs_diff(other.y) <= 1
    }
}

/// One crossing point of an entrance between two adjacent clusters: a pair of
/// mutually adjacent passable cells, one on each side of the border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// The side in the cluster nearer the grid origin — left of a vertical
    /// border, above a horizontal one.
    pub a: CellPos,
    /// The side in the adjacent cluster.
    pub b: CellPos,
}

/// The side of a cluster a border belongs to. Every border is keyed under the
/// cluster nearer the grid origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Right,
    Down,
}

/// One cluster border: the key every stored entrance hangs off.
type BorderKey = (ClusterPos, Side);

/// One cached intra-cluster cost's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct IntraKey {
    /// The mover shape the cost was searched for.
    shape: MoverShape,
    /// The cluster the search was confined to.
    cluster: ClusterPos,
    /// The lesser cell of the pair — costs are symmetric, so the pair is
    /// stored in ascending order.
    low: CellPos,
    /// The greater cell of the pair.
    high: CellPos,
}

/// The abstraction of one mover shape over the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerHierarchy {
    /// The mover shape this abstraction serves; a cell counts as passable when
    /// the shape's whole footprint anchored there is statically free on every
    /// layer of its mask. Two footprint sizes on the same layers therefore need
    /// separate abstractions — a wide mover's map genuinely has fewer ways
    /// through it.
    shape: MoverShape,
    /// Each border's transitions, keyed per border so one border re-derives
    /// without touching the rest. Borders without entrances hold no entry.
    transitions: BTreeMap<BorderKey, Vec<Transition>>,
    /// `regions[y * width + x]` — the connectivity region of that cell, or
    /// [`NO_REGION`] where impassable. Two cells connect if and only if their
    /// regions are equal.
    regions: Vec<u32>,
    /// How many regions the flood fill discovered.
    region_count: u32,
}

/// The hierarchical abstraction of a [`NavGrid`], built per mover shape:
/// fixed-size square clusters, the transitions crossing their borders, and
/// flood-filled connectivity regions over the cells.
///
/// The hierarchy sees the grid's **static** plane only — terrain and standing
/// footprints, never unit claims. It is derived state: writers of static
/// occupancy report their cells through [`mark_dirty`](Self::mark_dirty), and
/// one [`refresh`](Self::refresh) call at a fixed point of the game tick
/// folds the changes in. Queries never mutate the hierarchy's observable
/// state; the intra-cluster cost cache behind them memoizes pure functions of
/// the grid, so its fill timing cannot diverge between peers.
#[derive(Debug)]
pub struct NavHierarchy {
    /// Cells per cluster side. Clusters on the far edges of a grid whose
    /// dimensions are not multiples of this are cut short.
    cluster_size: u32,
    /// Grid width in cells.
    width: u32,
    /// Grid height in cells.
    height: u32,
    /// One abstraction per mover shape, in the order the shapes were given.
    layers: Vec<LayerHierarchy>,
    /// Borders whose entrances must be re-derived at the next refresh.
    dirty_borders: BTreeSet<BorderKey>,
    /// Clusters whose interior pathability changed since the last refresh;
    /// their cached intra-cluster costs are evicted on refresh.
    dirty_clusters: BTreeSet<ClusterPos>,
    /// Whether any static pathability changed since the last refresh.
    dirty: bool,
    /// Lazily computed shortest-path costs between two cells inside one
    /// cluster, `None` for pairs the cluster does not connect. Filled the
    /// first time a search needs an entry, evicted when the cluster dirties.
    ///
    /// The `Mutex` is interior mutability, not concurrency: searches memoize
    /// through a shared borrow, and the host resource must stay `Sync`
    /// (which rules `RefCell` out). Accesses are single-threaded, so the
    /// lock is never contended.
    intra_costs: Mutex<BTreeMap<IntraKey, Option<u32>>>,
}

/// Equality covers the derived structure; the cost cache is a memo of pure
/// grid functions, not identity.
impl PartialEq for NavHierarchy {
    fn eq(&self, other: &Self) -> bool {
        self.cluster_size == other.cluster_size
            && self.width == other.width
            && self.height == other.height
            && self.layers == other.layers
            && self.dirty_borders == other.dirty_borders
            && self.dirty_clusters == other.dirty_clusters
            && self.dirty == other.dirty
    }
}

impl Eq for NavHierarchy {}

impl Clone for NavHierarchy {
    fn clone(&self) -> Self {
        Self {
            cluster_size: self.cluster_size,
            width: self.width,
            height: self.height,
            layers: self.layers.clone(),
            dirty_borders: self.dirty_borders.clone(),
            dirty_clusters: self.dirty_clusters.clone(),
            dirty: self.dirty,
            intra_costs: Mutex::new(self.intra_costs.lock().unwrap().clone()),
        }
    }
}

impl NavHierarchy {
    /// Builds the hierarchy for the given mover shapes from the grid's current
    /// static occupancy.
    ///
    /// Panics if `cluster_size` is zero.
    pub fn build(grid: &NavGrid, cluster_size: u32, shapes: &[MoverShape]) -> Self {
        assert!(cluster_size > 0, "cluster size must be greater than 0");

        let layers = shapes
            .iter()
            .map(|&shape| LayerHierarchy::build(grid, cluster_size, shape))
            .collect();

        Self {
            cluster_size,
            width: grid.width(),
            height: grid.height(),
            layers,
            dirty_borders: BTreeSet::new(),
            dirty_clusters: BTreeSet::new(),
            dirty: false,
            intra_costs: Mutex::new(BTreeMap::new()),
        }
    }

    /// Records that the static pathability of `cell` changed, queueing its
    /// cluster borders for the next [`refresh`](Self::refresh).
    ///
    /// Out-of-bounds cells are silently ignored.
    pub fn mark_dirty(&mut self, cell: CellPos) {
        if cell.x >= self.width || cell.y >= self.height {
            return;
        }
        self.dirty = true;

        // A changed cell also changes whether footprints *anchored before it*
        // fit, as far back as the widest served shape reaches — so every anchor
        // whose footprint covers the cell dirties its own borders and cluster,
        // or a wide shape's transitions would go stale from changes landing
        // just past a border.
        let (reach_x, reach_y) = self.layers.iter().fold((0, 0), |(x, y), layer| {
            (
                x.max(layer.shape.size.width - 1),
                y.max(layer.shape.size.height - 1),
            )
        });
        for dy in 0..=reach_y {
            for dx in 0..=reach_x {
                if let (Some(x), Some(y)) = (cell.x.checked_sub(dx), cell.y.checked_sub(dy)) {
                    self.mark_anchor_dirty(CellPos::new(x, y));
                }
            }
        }
    }

    /// Queues the borders and cluster one anchor cell touches.
    fn mark_anchor_dirty(&mut self, cell: CellPos) {
        let size = self.cluster_size;
        let cluster = self.cluster_of(cell);
        self.dirty_clusters.insert(cluster);
        if cell.x % size == size - 1 && cell.x + 1 < self.width {
            self.dirty_borders.insert((cluster, Side::Right));
        }
        if cell.x.is_multiple_of(size) && cluster.x > 0 {
            let left = ClusterPos {
                x: cluster.x - 1,
                y: cluster.y,
            };
            self.dirty_borders.insert((left, Side::Right));
        }
        if cell.y % size == size - 1 && cell.y + 1 < self.height {
            self.dirty_borders.insert((cluster, Side::Down));
        }
        if cell.y.is_multiple_of(size) && cluster.y > 0 {
            let above = ClusterPos {
                x: cluster.x,
                y: cluster.y - 1,
            };
            self.dirty_borders.insert((above, Side::Down));
        }
    }

    /// Folds every change reported since the last refresh into the hierarchy:
    /// dirty borders re-derive their entrances, and regions reflood. A no-op
    /// when nothing was reported.
    ///
    /// This is the hierarchy's only mutation point after the build.
    pub fn refresh(&mut self, grid: &NavGrid) {
        if !self.dirty {
            return;
        }

        for layer in &mut self.layers {
            for &border in &self.dirty_borders {
                let entrance = border_transitions(grid, layer.shape, self.cluster_size, border);
                if entrance.is_empty() {
                    layer.transitions.remove(&border);
                } else {
                    layer.transitions.insert(border, entrance);
                }
            }
            let (regions, region_count) = flood_regions(grid, layer.shape);
            layer.regions = regions;
            layer.region_count = region_count;
        }

        self.intra_costs
            .lock()
            .unwrap()
            .retain(|key, _| !self.dirty_clusters.contains(&key.cluster));

        self.dirty_borders.clear();
        self.dirty_clusters.clear();
        self.dirty = false;
    }

    /// Returns the cells per cluster side.
    pub fn cluster_size(&self) -> u32 {
        self.cluster_size
    }

    /// Returns the cluster containing `cell`.
    pub fn cluster_of(&self, cell: CellPos) -> ClusterPos {
        ClusterPos {
            x: cell.x / self.cluster_size,
            y: cell.y / self.cluster_size,
        }
    }

    /// Returns the cells `cluster` covers; far-edge clusters are cut short
    /// by the grid bounds.
    pub fn cluster_rect(&self, cluster: ClusterPos) -> CellRect {
        let origin = CellPos::new(cluster.x * self.cluster_size, cluster.y * self.cluster_size);
        let width = self.cluster_size.min(self.width - origin.x);
        let height = self.cluster_size.min(self.height - origin.y);
        CellRect::new(origin, CellSize::new(width, height))
    }

    /// Returns each transition touching `cluster`, paired with the side cell
    /// that lies inside it, in border order.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub(crate) fn transition_sides(
        &self,
        shape: MoverShape,
        cluster: ClusterPos,
    ) -> Vec<(CellPos, Transition)> {
        let layer = self.layer(shape);
        let mut sides = Vec::new();

        // The cluster's own right/down borders hold its `a` sides; the left
        // and upper neighbors' borders hold its `b` sides.
        let mut collect = |border: BorderKey, take_a: bool| {
            if let Some(transitions) = layer.transitions.get(&border) {
                for &transition in transitions {
                    let side = if take_a { transition.a } else { transition.b };
                    sides.push((side, transition));
                }
            }
        };

        if cluster.x > 0 {
            let left = ClusterPos {
                x: cluster.x - 1,
                y: cluster.y,
            };
            collect((left, Side::Right), false);
        }
        if cluster.y > 0 {
            let above = ClusterPos {
                x: cluster.x,
                y: cluster.y - 1,
            };
            collect((above, Side::Down), false);
        }
        collect((cluster, Side::Right), true);
        collect((cluster, Side::Down), true);

        sides
    }

    /// Returns the cost of the cheapest path between `a` and `b` that stays
    /// inside `cluster` on the static plane, or `None` when the cluster does
    /// not connect them. Computed on first use and cached until the cluster
    /// dirties.
    pub(crate) fn intra_cost(
        &self,
        grid: &NavGrid,
        projection: Projection,
        shape: MoverShape,
        cluster: ClusterPos,
        a: CellPos,
        b: CellPos,
    ) -> Option<u32> {
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        let key = IntraKey {
            shape,
            cluster,
            low,
            high,
        };

        if let Some(&cost) = self.intra_costs.lock().unwrap().get(&key) {
            return cost;
        }

        let window = self.cluster_rect(cluster);
        let cost = astar::bounded_cost(
            grid,
            projection,
            window,
            low,
            MoverProfile::new(shape, Blockers::Static),
            high,
        );
        self.intra_costs.lock().unwrap().insert(key, cost);
        cost
    }

    /// Returns every transition of the given mover shape, grouped per border,
    /// borders in cluster order.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn transitions(&self, shape: MoverShape) -> impl Iterator<Item = Transition> {
        self.layer(shape).transitions.values().flatten().copied()
    }

    /// Returns the connectivity region of `cell` for the given mover shape, or
    /// `None` when the cell is impassable or out of bounds.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn region_of(&self, cell: CellPos, shape: MoverShape) -> Option<u32> {
        if cell.x >= self.width || cell.y >= self.height {
            return None;
        }
        let region = self.layer(shape).regions[(cell.y * self.width + cell.x) as usize];
        (region != NO_REGION).then_some(region)
    }

    /// Returns `true` when `a` and `b` are connected for the given mover shape:
    /// both passable and in the same region.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn same_region(&self, a: CellPos, shape: MoverShape, b: CellPos) -> bool {
        match (self.region_of(a, shape), self.region_of(b, shape)) {
            (Some(region_a), Some(region_b)) => region_a == region_b,
            (None, _) | (_, None) => false,
        }
    }

    /// Returns how many connectivity regions the given mover shape has.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn region_count(&self, shape: MoverShape) -> u32 {
        self.layer(shape).region_count
    }

    /// Returns `true` when an abstraction was built for the given mover shape.
    pub fn serves(&self, shape: MoverShape) -> bool {
        self.layers.iter().any(|layer| layer.shape == shape)
    }

    /// Every mover shape this hierarchy serves, in the order they were given.
    pub fn shapes(&self) -> impl Iterator<Item = MoverShape> {
        self.layers.iter().map(|layer| layer.shape)
    }

    /// The abstraction serving `shape`.
    fn layer(&self, shape: MoverShape) -> &LayerHierarchy {
        self.layers
            .iter()
            .find(|layer| layer.shape == shape)
            .unwrap_or_else(|| {
                panic!(
                    "no hierarchy built for shape {} at {}x{}",
                    shape.mask, shape.size.width, shape.size.height
                )
            })
    }
}

impl LayerHierarchy {
    /// Builds one shape's abstraction from the grid's current static
    /// occupancy.
    fn build(grid: &NavGrid, cluster_size: u32, shape: MoverShape) -> Self {
        let mut transitions = BTreeMap::new();
        let clusters_w = grid.width().div_ceil(cluster_size);
        let clusters_h = grid.height().div_ceil(cluster_size);

        for cy in 0..clusters_h {
            for cx in 0..clusters_w {
                let cluster = ClusterPos { x: cx, y: cy };
                for side in [Side::Right, Side::Down] {
                    let border = (cluster, side);
                    if !border_exists(grid, cluster_size, border) {
                        continue;
                    }
                    let entrance = border_transitions(grid, shape, cluster_size, border);
                    if !entrance.is_empty() {
                        transitions.insert(border, entrance);
                    }
                }
            }
        }

        let (regions, region_count) = flood_regions(grid, shape);
        Self {
            shape,
            transitions,
            regions,
            region_count,
        }
    }
}

/// Whether the border lies inside the grid — far-edge clusters have no right
/// or bottom neighbor.
fn border_exists(grid: &NavGrid, cluster_size: u32, (cluster, side): BorderKey) -> bool {
    match side {
        Side::Right => (cluster.x + 1) * cluster_size < grid.width(),
        Side::Down => (cluster.y + 1) * cluster_size < grid.height(),
    }
}

/// Derives one border's transitions from the grid: a maximal run of border
/// cell pairs statically passable on both sides yields one transition in its
/// middle, or one at each end when at least [`WIDE_ENTRANCE`] cells wide.
fn border_transitions(
    grid: &NavGrid,
    shape: MoverShape,
    cluster_size: u32,
    (cluster, side): BorderKey,
) -> Vec<Transition> {
    let mut transitions = Vec::new();
    let mut run: Vec<Transition> = Vec::new();

    let pairs: Vec<Transition> = match side {
        Side::Right => {
            let border_x = (cluster.x + 1) * cluster_size;
            let y_end = ((cluster.y + 1) * cluster_size).min(grid.height());
            (cluster.y * cluster_size..y_end)
                .map(|y| Transition {
                    a: CellPos::new(border_x - 1, y),
                    b: CellPos::new(border_x, y),
                })
                .collect()
        }
        Side::Down => {
            let border_y = (cluster.y + 1) * cluster_size;
            let x_end = ((cluster.x + 1) * cluster_size).min(grid.width());
            (cluster.x * cluster_size..x_end)
                .map(|x| Transition {
                    a: CellPos::new(x, border_y - 1),
                    b: CellPos::new(x, border_y),
                })
                .collect()
        }
    };

    for pair in pairs {
        if grid.fits_statically(pair.a, shape) && grid.fits_statically(pair.b, shape) {
            run.push(pair);
        } else {
            emit_entrance(&run, &mut transitions);
            run.clear();
        }
    }
    emit_entrance(&run, &mut transitions);

    transitions
}

/// Appends one entrance's transitions: the middle pair of a narrow run, the
/// two end pairs of a wide one.
fn emit_entrance(run: &[Transition], transitions: &mut Vec<Transition>) {
    match run.len() {
        0 => {}
        len if (len as u32) < WIDE_ENTRANCE => transitions.push(run[len / 2]),
        len => {
            transitions.push(run[0]);
            transitions.push(run[len - 1]);
        }
    }
}

/// Flood-fills connectivity regions over every cell of the static plane,
/// using the same neighbor rule as the search, and returns them with their
/// count.
///
/// Cells are seeded in row-major order and each fill is a breadth-first walk
/// in fixed direction order, so region numbering is deterministic.
fn flood_regions(grid: &NavGrid, shape: MoverShape) -> (Vec<u32>, u32) {
    let width = grid.width();
    let index = |pos: CellPos| (pos.y * width + pos.x) as usize;

    let mut regions = vec![NO_REGION; width as usize * grid.height() as usize];
    let mut count = 0;

    for y in 0..grid.height() {
        for x in 0..width {
            let seed = CellPos::new(x, y);
            if regions[index(seed)] != NO_REGION || !grid.fits_statically(seed, shape) {
                continue;
            }

            regions[index(seed)] = count;
            let mut queue = VecDeque::from([seed]);
            while let Some(pos) = queue.pop_front() {
                for (neighbor, _) in
                    astar::passable_neighbors(grid, pos, MoverProfile::new(shape, Blockers::Static))
                {
                    if regions[index(neighbor)] == NO_REGION {
                        regions[index(neighbor)] = count;
                        queue.push_back(neighbor);
                    }
                }
            }
            count += 1;
        }
    }

    (regions, count)
}
