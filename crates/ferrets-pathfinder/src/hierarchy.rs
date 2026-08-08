//! Hierarchical view of the navigation grid: clusters, entrance transitions,
//! and connectivity regions.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Mutex,
};

use crate::{
    astar::{self, Blockers},
    layer_mask::LayerMask,
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

/// One cached intra-cluster cost: the mover mask's bits, the cluster, and the
/// cell pair in ascending order (costs are symmetric).
type IntraKey = (u32, ClusterPos, CellPos, CellPos);

/// The abstraction of one mover mask over the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerHierarchy {
    /// The mover mask this abstraction serves; a cell counts as passable when
    /// it is statically free on every layer in it.
    mask: LayerMask,
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

/// The hierarchical abstraction of a [`NavGrid`], built per mover layer mask:
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
    /// One abstraction per mover mask, in the order the masks were given.
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
    /// Builds the hierarchy for the given mover masks from the grid's current
    /// static occupancy.
    ///
    /// Panics if `cluster_size` is zero.
    pub fn build(grid: &NavGrid, cluster_size: u32, masks: &[LayerMask]) -> Self {
        assert!(cluster_size > 0, "cluster size must be greater than 0");

        let layers = masks
            .iter()
            .map(|&mask| LayerHierarchy::build(grid, cluster_size, mask))
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
                let entrance = border_transitions(grid, layer.mask, self.cluster_size, border);
                if entrance.is_empty() {
                    layer.transitions.remove(&border);
                } else {
                    layer.transitions.insert(border, entrance);
                }
            }
            let (regions, region_count) = flood_regions(grid, layer.mask);
            layer.regions = regions;
            layer.region_count = region_count;
        }

        self.intra_costs
            .lock()
            .unwrap()
            .retain(|(_, cluster, _, _), _| !self.dirty_clusters.contains(cluster));

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
    /// Panics if no hierarchy was built for the mask.
    pub(crate) fn transition_sides(
        &self,
        mask: LayerMask,
        cluster: ClusterPos,
    ) -> Vec<(CellPos, Transition)> {
        let layer = self.layer(mask);
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
        mask: LayerMask,
        cluster: ClusterPos,
        a: CellPos,
        b: CellPos,
    ) -> Option<u32> {
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        let key = (*mask, cluster, low, high);

        if let Some(&cost) = self.intra_costs.lock().unwrap().get(&key) {
            return cost;
        }

        let window = self.cluster_rect(cluster);
        let cost = astar::bounded_cost(grid, projection, mask, Blockers::Static, window, low, high);
        self.intra_costs.lock().unwrap().insert(key, cost);
        cost
    }

    /// Returns every transition of the given mover mask, grouped per border,
    /// borders in cluster order.
    ///
    /// Panics if no hierarchy was built for the mask.
    pub fn transitions(&self, mask: impl Into<LayerMask>) -> impl Iterator<Item = Transition> {
        self.layer(mask.into())
            .transitions
            .values()
            .flatten()
            .copied()
    }

    /// Returns the connectivity region of `cell` for the given mover mask, or
    /// `None` when the cell is impassable or out of bounds.
    ///
    /// Panics if no hierarchy was built for the mask.
    pub fn region_of(&self, mask: impl Into<LayerMask>, cell: CellPos) -> Option<u32> {
        if cell.x >= self.width || cell.y >= self.height {
            return None;
        }
        let region = self.layer(mask.into()).regions[(cell.y * self.width + cell.x) as usize];
        (region != NO_REGION).then_some(region)
    }

    /// Returns `true` when `a` and `b` are connected for the given mover mask:
    /// both passable and in the same region.
    ///
    /// Panics if no hierarchy was built for the mask.
    pub fn same_region(&self, mask: impl Into<LayerMask>, a: CellPos, b: CellPos) -> bool {
        let mask = mask.into();
        match (self.region_of(mask, a), self.region_of(mask, b)) {
            (Some(region_a), Some(region_b)) => region_a == region_b,
            (None, _) | (_, None) => false,
        }
    }

    /// Returns how many connectivity regions the given mover mask has.
    ///
    /// Panics if no hierarchy was built for the mask.
    pub fn region_count(&self, mask: impl Into<LayerMask>) -> u32 {
        self.layer(mask.into()).region_count
    }

    /// Returns `true` when an abstraction was built for the given mover mask.
    pub fn serves(&self, mask: impl Into<LayerMask>) -> bool {
        let mask = mask.into();
        self.layers.iter().any(|layer| layer.mask == mask)
    }

    /// The abstraction serving `mask`.
    fn layer(&self, mask: LayerMask) -> &LayerHierarchy {
        self.layers
            .iter()
            .find(|layer| layer.mask == mask)
            .unwrap_or_else(|| panic!("no hierarchy built for mask {mask}"))
    }
}

impl LayerHierarchy {
    /// Builds one mask's abstraction from the grid's current static
    /// occupancy.
    fn build(grid: &NavGrid, cluster_size: u32, mask: LayerMask) -> Self {
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
                    let entrance = border_transitions(grid, mask, cluster_size, border);
                    if !entrance.is_empty() {
                        transitions.insert(border, entrance);
                    }
                }
            }
        }

        let (regions, region_count) = flood_regions(grid, mask);
        Self {
            mask,
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
    mask: LayerMask,
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
        if grid.is_statically_passable_by(mask, pair.a)
            && grid.is_statically_passable_by(mask, pair.b)
        {
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
fn flood_regions(grid: &NavGrid, mask: LayerMask) -> (Vec<u32>, u32) {
    let width = grid.width();
    let index = |pos: CellPos| (pos.y * width + pos.x) as usize;

    let mut regions = vec![NO_REGION; width as usize * grid.height() as usize];
    let mut count = 0;

    for y in 0..grid.height() {
        for x in 0..width {
            let seed = CellPos::new(x, y);
            if regions[index(seed)] != NO_REGION || !grid.is_statically_passable_by(mask, seed) {
                continue;
            }

            regions[index(seed)] = count;
            let mut queue = VecDeque::from([seed]);
            while let Some(pos) = queue.pop_front() {
                for (neighbor, _) in astar::passable_neighbors(grid, mask, pos, Blockers::Static) {
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
