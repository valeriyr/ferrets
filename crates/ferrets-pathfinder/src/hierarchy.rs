//! Hierarchical view of the navigation grid: clusters, entrance transitions,
//! and connectivity regions.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::Mutex,
};

use crate::{
    astar,
    mover_profile::{Blockers, MoverProfile},
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_nav_grid::{layer_mask::LayerMask, mover_shape::MoverShape, nav_grid::NavGrid};

/// Cells per cluster side used by the live game grid.
pub const DEFAULT_CLUSTER_SIZE: u32 = 16;

/// Entrances at least this many cells wide get a transition at each end
/// instead of a single one in the middle.
const WIDE_ENTRANCE: u32 = 6;

/// Local region value marking a cell the shape does not fit.
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

/// One cluster's cached intra-cluster costs for one shape, keyed by the pair
/// of cells in ascending order.
type ClusterCosts = HashMap<(CellPos, CellPos), Option<u32>>;

/// The abstraction of one mover shape over the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShapeHierarchy {
    /// The mover shape this abstraction serves; a cell counts as passable when
    /// the shape's whole footprint anchored there is statically clear for it.
    shape: MoverShape,
    /// Each border's transitions, keyed per border: one entrance per run of
    /// crossable pairs joining the same two local regions. Borders without
    /// entrances hold no entry.
    transitions: BTreeMap<BorderKey, Vec<Transition>>,
    /// `locals[y * width + x]` — the region of that cell within its cluster,
    /// or [`NO_REGION`] where the shape does not fit.
    locals: Vec<u32>,
    /// Each cluster's count of local regions, clusters in row-major order.
    local_counts: Vec<usize>,
    /// Each border's links: the pairs of local regions — the keyed
    /// cluster's first — that a cardinal step across the border joins,
    /// sorted. Borders without links hold no entry.
    links: BTreeMap<BorderKey, Vec<(usize, usize)>>,
    /// Each cluster's connectivity region per local region, clusters in
    /// row-major order. Two cells connect if and only if their regions are
    /// equal.
    labels: Vec<Vec<u32>>,
    /// How many connectivity regions the shape has.
    region_count: u32,
    /// Clusters whose cells' fit may have changed since the last refresh.
    dirty_clusters: BTreeSet<ClusterPos>,
}

/// The hierarchical abstraction of a [`NavGrid`], built per mover shape:
/// fixed-size square clusters, the transitions crossing their borders, and
/// flood-filled connectivity regions over the cells.
///
/// The hierarchy sees the grid's **static** plane only — terrain, relief and
/// standing footprints, never unit claims. It is derived state: writers of static
/// occupancy report their cells through [`mark_dirty`](Self::mark_dirty), and
/// one [`refresh`](Self::refresh) call at a fixed point of the game tick
/// folds the changes in. Queries never mutate the hierarchy's observable
/// state; the intra-cluster cost cache behind them holds costs computed on
/// the grid as it stands.
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
    abstractions: Vec<ShapeHierarchy>,
    /// Lazily computed shortest-path costs between two cells inside one
    /// cluster, per shape in the abstractions' order, then per cluster in
    /// row-major order, keyed by the pair in ascending order; `None` for
    /// pairs the cluster does not connect. Filled the first time a search
    /// needs an entry; a shape's entries for a cluster are cleared when a
    /// change marks the cluster for it.
    ///
    /// Behind a `Mutex` for interior mutability; accesses are
    /// single-threaded, so it is never contended.
    intra_costs: Mutex<Vec<Vec<ClusterCosts>>>,
}

/// Equality covers the derived structure; the cost cache is a memo of pure
/// grid functions, not identity.
impl PartialEq for NavHierarchy {
    fn eq(&self, other: &Self) -> bool {
        self.cluster_size == other.cluster_size
            && self.width == other.width
            && self.height == other.height
            && self.abstractions == other.abstractions
    }
}

impl Eq for NavHierarchy {}

impl Clone for NavHierarchy {
    fn clone(&self) -> Self {
        Self {
            cluster_size: self.cluster_size,
            width: self.width,
            height: self.height,
            abstractions: self.abstractions.clone(),
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

        let abstractions = shapes
            .iter()
            .map(|&shape| ShapeHierarchy::build(grid, cluster_size, shape))
            .collect();
        let cluster_count = cluster_count(grid, cluster_size);

        Self {
            cluster_size,
            width: grid.width(),
            height: grid.height(),
            abstractions,
            intra_costs: Mutex::new(vec![vec![HashMap::new(); cluster_count]; shapes.len()]),
        }
    }

    /// Records that the static occupancy of `cell` changed on `layers`,
    /// queueing for the next [`refresh`](Self::refresh) the clusters of every
    /// shape whose fit it may change — each shape that must be free on one of
    /// `layers`, for every anchor its footprint reaches the cell from — and
    /// clearing that shape's cached intra-cluster costs in them.
    ///
    /// Panics for a cell off the grid or an empty mask.
    pub fn mark_dirty(&mut self, cell: CellPos, layers: impl Into<LayerMask>) {
        assert!(
            cell.x < self.width && cell.y < self.height,
            "a static change lies on the grid"
        );
        let layers = layers.into();
        assert!(
            layers != LayerMask::EMPTY,
            "a static change lies on some layer"
        );
        let clusters_wide = self.width.div_ceil(self.cluster_size);
        let costs = self.intra_costs.get_mut().unwrap();
        for (abstraction, costs) in self.abstractions.iter_mut().zip(costs.iter_mut()) {
            if abstraction.shape.layers & layers == LayerMask::EMPTY {
                continue;
            }
            let size = abstraction.shape.size;
            for dy in 0..size.height {
                for dx in 0..size.width {
                    if let (Some(x), Some(y)) = (cell.x.checked_sub(dx), cell.y.checked_sub(dy)) {
                        let cluster = ClusterPos {
                            x: x / self.cluster_size,
                            y: y / self.cluster_size,
                        };
                        abstraction.dirty_clusters.insert(cluster);
                        costs[cluster_index(cluster, clusters_wide)].clear();
                    }
                }
            }
        }
    }

    /// Folds every change reported since the last refresh into the hierarchy,
    /// shape by shape: the dirty clusters reflood their local regions, every
    /// border they touch re-derives its entrances and links, and the shape's
    /// regions are relabeled. A shape with nothing reported is left as it is.
    pub fn refresh(&mut self, grid: &NavGrid) {
        for abstraction in &mut self.abstractions {
            abstraction.refresh(grid, self.cluster_size);
        }
    }

    /// Whether a change was reported for any shape since the last
    /// [`refresh`](Self::refresh), so regions may not reflect the grid.
    pub fn is_dirty(&self) -> bool {
        self.abstractions
            .iter()
            .any(|abstraction| abstraction.is_dirty())
    }

    /// Whether a change was reported for `shape` since the last
    /// [`refresh`](Self::refresh), so its regions may not reflect the grid.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn is_dirty_for(&self, shape: MoverShape) -> bool {
        self.abstraction(shape).is_dirty()
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
        cluster_rect(self.cluster_size, self.width, self.height, cluster)
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
        let abstraction = self.abstraction(shape);
        let mut sides = Vec::new();

        // The cluster's own right/down borders hold its `a` sides; the left
        // and upper neighbors' borders hold its `b` sides.
        let mut collect = |border: BorderKey, take_a: bool| {
            if let Some(transitions) = abstraction.transitions.get(&border) {
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
    /// not connect them. Computed on first use and cached until a change marks
    /// the cluster for the shape.
    pub(crate) fn intra_cost(
        &self,
        grid: &NavGrid,
        projection: Projection,
        shape: MoverShape,
        cluster: ClusterPos,
        a: CellPos,
        b: CellPos,
    ) -> Option<u32> {
        let index = self.abstraction_index(shape);
        let abstraction = &self.abstractions[index];
        // On a cluster refreshed for the shape, the bounded search confined
        // to it connects exactly the cells of one local region.
        if !abstraction.dirty_clusters.contains(&cluster) {
            match (
                abstraction.local_of(a, self.width),
                abstraction.local_of(b, self.width),
            ) {
                (Some(local_a), Some(local_b)) if local_a == local_b => {}
                (Some(_) | None, Some(_) | None) => return None,
            }
        }
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        let cluster_index = cluster_index(cluster, self.width.div_ceil(self.cluster_size));

        if let Some(&cost) =
            self.intra_costs.lock().unwrap()[index][cluster_index].get(&(low, high))
        {
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
        self.intra_costs.lock().unwrap()[index][cluster_index].insert((low, high), cost);
        cost
    }

    /// Returns every transition of the given mover shape, grouped per border,
    /// borders in cluster order.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn transitions(&self, shape: MoverShape) -> impl Iterator<Item = Transition> {
        self.abstraction(shape)
            .transitions
            .values()
            .flatten()
            .copied()
    }

    /// Returns the connectivity region of `cell` for the given mover shape, or
    /// `None` when the cell is impassable or out of bounds. A region id holds
    /// until the next [`refresh`](Self::refresh), which may renumber it.
    ///
    /// Panics if no hierarchy was built for the shape.
    pub fn region_of(&self, cell: CellPos, shape: MoverShape) -> Option<u32> {
        if cell.x >= self.width || cell.y >= self.height {
            return None;
        }
        let abstraction = self.abstraction(shape);
        let local = abstraction.local_of(cell, self.width)?;
        let cluster = self.cluster_of(cell);
        let clusters_wide = self.width.div_ceil(self.cluster_size);
        Some(abstraction.labels[cluster_index(cluster, clusters_wide)][local])
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
        self.abstraction(shape).region_count
    }

    /// Returns `true` when an abstraction was built for the given mover shape.
    pub fn serves(&self, shape: MoverShape) -> bool {
        self.abstractions
            .iter()
            .any(|abstraction| abstraction.shape == shape)
    }

    /// Every mover shape this hierarchy serves, in the order they were given.
    pub fn shapes(&self) -> impl Iterator<Item = MoverShape> {
        self.abstractions
            .iter()
            .map(|abstraction| abstraction.shape)
    }

    /// The abstraction serving `shape`.
    fn abstraction(&self, shape: MoverShape) -> &ShapeHierarchy {
        &self.abstractions[self.abstraction_index(shape)]
    }

    /// The index of the abstraction serving `shape`.
    fn abstraction_index(&self, shape: MoverShape) -> usize {
        self.abstractions
            .iter()
            .position(|abstraction| abstraction.shape == shape)
            .unwrap_or_else(|| {
                panic!(
                    "no hierarchy built for shape on terrains {} and layers {} at {}x{}, cliff crossing {:?}",
                    shape.terrains, shape.layers, shape.size.width, shape.size.height, shape.cliff_crossing
                )
            })
    }
}

impl ShapeHierarchy {
    /// Builds one shape's abstraction from the grid's current static
    /// occupancy: every cluster dirty, then refreshed.
    fn build(grid: &NavGrid, cluster_size: u32, shape: MoverShape) -> Self {
        let clusters_wide = grid.width().div_ceil(cluster_size);
        let clusters_high = grid.height().div_ceil(cluster_size);
        let mut abstraction = Self {
            shape,
            transitions: BTreeMap::new(),
            locals: vec![NO_REGION; cell_index(CellPos::new(0, grid.height()), grid.width())],
            local_counts: vec![0; cluster_count(grid, cluster_size)],
            links: BTreeMap::new(),
            labels: vec![Vec::new(); cluster_count(grid, cluster_size)],
            region_count: 0,
            dirty_clusters: (0..clusters_high)
                .flat_map(|y| (0..clusters_wide).map(move |x| ClusterPos { x, y }))
                .collect(),
        };
        abstraction.refresh(grid, cluster_size);
        abstraction
    }

    /// Whether a change was reported for this shape since its last refresh.
    fn is_dirty(&self) -> bool {
        !self.dirty_clusters.is_empty()
    }

    /// Folds this shape's reported changes in: the dirty clusters reflood,
    /// every border they touch re-derives its entrances and links, and the
    /// regions are relabeled over the whole graph. Nothing when no change was
    /// reported.
    fn refresh(&mut self, grid: &NavGrid, cluster_size: u32) {
        if !self.is_dirty() {
            return;
        }
        let dirty_clusters = std::mem::take(&mut self.dirty_clusters);
        for &cluster in &dirty_clusters {
            self.flood_cluster(grid, cluster_size, cluster);
        }
        let touched: BTreeSet<BorderKey> = dirty_clusters
            .iter()
            .flat_map(|&cluster| borders_of(cluster))
            .filter(|&border| border_exists(grid, cluster_size, border))
            .collect();
        for border in touched {
            let entrance = self.border_transitions(grid, cluster_size, border);
            if entrance.is_empty() {
                self.transitions.remove(&border);
            } else {
                self.transitions.insert(border, entrance);
            }
            let links = self.border_links(grid, cluster_size, border);
            if links.is_empty() {
                self.links.remove(&border);
            } else {
                self.links.insert(border, links);
            }
        }
        self.relabel(grid.width().div_ceil(cluster_size));
    }

    /// Refloods `cluster`'s local regions with the search's own neighbor
    /// rule, confined to the cluster. Cells are seeded in row-major order and
    /// each fill is a breadth-first walk in fixed direction order.
    fn flood_cluster(&mut self, grid: &NavGrid, cluster_size: u32, cluster: ClusterPos) {
        let width = grid.width();
        let index = |pos: CellPos| cell_index(pos, width);
        let rect = cluster_rect(cluster_size, width, grid.height(), cluster);
        for cell in rect.cells() {
            self.locals[index(cell)] = NO_REGION;
        }

        let profile = MoverProfile::new(self.shape, Blockers::Static);
        let mut count = 0;
        for seed in rect.cells() {
            if self.locals[index(seed)] != NO_REGION || !grid.fits_statically(seed, self.shape) {
                continue;
            }
            self.locals[index(seed)] = count;
            let mut queue = VecDeque::from([seed]);
            while let Some(pos) = queue.pop_front() {
                for (neighbor, _) in astar::passable_neighbors(grid, pos, profile) {
                    if rect.contains(neighbor) && self.locals[index(neighbor)] == NO_REGION {
                        self.locals[index(neighbor)] = count;
                        queue.push_back(neighbor);
                    }
                }
            }
            count += 1;
        }
        self.local_counts[cluster_index(cluster, width.div_ceil(cluster_size))] = count as usize;
    }

    /// Derives one border's transitions: a maximal run of border pairs
    /// joining the same two local regions yields one transition in its
    /// middle, or one at each end when at least [`WIDE_ENTRANCE`] cells wide.
    fn border_transitions(
        &self,
        grid: &NavGrid,
        cluster_size: u32,
        border: BorderKey,
    ) -> Vec<Transition> {
        let mut transitions = Vec::new();
        let mut run: Vec<Transition> = Vec::new();
        let mut joined: Option<(usize, usize)> = None;
        for pair in border_pairs(grid, cluster_size, border) {
            let crossing = self.crossing(grid, pair);
            if crossing != joined {
                emit_entrance(&run, &mut transitions);
                run.clear();
                joined = crossing;
            }
            if crossing.is_some() {
                run.push(pair);
            }
        }
        emit_entrance(&run, &mut transitions);
        transitions
    }

    /// Derives one border's links: the pairs of local regions a cardinal
    /// step across joins.
    fn border_links(
        &self,
        grid: &NavGrid,
        cluster_size: u32,
        border: BorderKey,
    ) -> Vec<(usize, usize)> {
        let mut links: Vec<(usize, usize)> = border_pairs(grid, cluster_size, border)
            .into_iter()
            .filter_map(|pair| self.crossing(grid, pair))
            .collect();
        links.sort_unstable();
        links.dedup();
        links
    }

    /// Labels the connectivity regions: the components of the graph of
    /// (cluster, local region) nodes joined by the border links, numbered in
    /// cluster order, then local order.
    fn relabel(&mut self, clusters_wide: u32) {
        let mut offsets = Vec::with_capacity(self.local_counts.len());
        let mut total = 0;
        for &count in &self.local_counts {
            offsets.push(total);
            total += count;
        }
        let node = |cluster: ClusterPos, local: usize| {
            offsets[cluster_index(cluster, clusters_wide)] + local
        };

        let mut parent: Vec<usize> = (0..total).collect();
        for (&(cluster, side), links) in &self.links {
            let other = match side {
                Side::Right => ClusterPos {
                    x: cluster.x + 1,
                    y: cluster.y,
                },
                Side::Down => ClusterPos {
                    x: cluster.x,
                    y: cluster.y + 1,
                },
            };
            for &(a, b) in links {
                let (root_a, root_b) = (
                    find_root(&mut parent, node(cluster, a)),
                    find_root(&mut parent, node(other, b)),
                );
                parent[root_a.max(root_b)] = root_a.min(root_b);
            }
        }

        let mut label_of_root: Vec<Option<u32>> = vec![None; total];
        let mut region_count = 0;
        for (cluster_index, &count) in self.local_counts.iter().enumerate() {
            let offset = offsets[cluster_index];
            self.labels[cluster_index] = (0..count)
                .map(|local| {
                    let root = find_root(&mut parent, offset + local);
                    match label_of_root[root] {
                        Some(label) => label,
                        None => {
                            label_of_root[root] = Some(region_count);
                            region_count += 1;
                            region_count - 1
                        }
                    }
                })
                .collect();
        }
        self.region_count = region_count;
    }

    /// The two local regions a cardinal step across `pair` joins, the keyed
    /// cluster's first, or `None` where the shape cannot step across.
    fn crossing(&self, grid: &NavGrid, pair: Transition) -> Option<(usize, usize)> {
        let width = grid.width();
        let a = self.local_of(pair.a, width)?;
        let b = self.local_of(pair.b, width)?;
        grid.can_step(pair.a, pair.b, self.shape).then_some((a, b))
    }

    /// The region of `cell` within its cluster, or `None` where the shape
    /// does not fit.
    fn local_of(&self, cell: CellPos, width: u32) -> Option<usize> {
        let local = self.locals[cell_index(cell, width)];
        (local != NO_REGION).then_some(local as usize)
    }
}

/// The index of `cell` in a row-major array over a grid `width` cells wide.
fn cell_index(cell: CellPos, width: u32) -> usize {
    cell.y as usize * width as usize + cell.x as usize
}

/// The index of `cluster` in a row-major array over a cluster grid
/// `clusters_wide` clusters wide.
fn cluster_index(cluster: ClusterPos, clusters_wide: u32) -> usize {
    cluster.y as usize * clusters_wide as usize + cluster.x as usize
}

/// How many clusters of `cluster_size` cover `grid`, far-edge ones cut short.
fn cluster_count(grid: &NavGrid, cluster_size: u32) -> usize {
    grid.width().div_ceil(cluster_size) as usize * grid.height().div_ceil(cluster_size) as usize
}

/// The root of `node`'s set in a union-find forest, halving the path on the
/// way.
fn find_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// The cells `cluster` covers on a `width` × `height` grid; far-edge
/// clusters are cut short by the grid bounds.
fn cluster_rect(cluster_size: u32, width: u32, height: u32, cluster: ClusterPos) -> CellRect {
    let origin = CellPos::new(cluster.x * cluster_size, cluster.y * cluster_size);
    let cut_width = cluster_size.min(width - origin.x);
    let cut_height = cluster_size.min(height - origin.y);
    CellRect::new(origin, CellSize::new(cut_width, cut_height))
}

/// Every border `cluster` lies on: its own right and down borders and its
/// left and upper neighbors', those past the grid's edge included.
fn borders_of(cluster: ClusterPos) -> Vec<BorderKey> {
    let mut borders = vec![(cluster, Side::Right), (cluster, Side::Down)];
    if cluster.x > 0 {
        let left = ClusterPos {
            x: cluster.x - 1,
            y: cluster.y,
        };
        borders.push((left, Side::Right));
    }
    if cluster.y > 0 {
        let above = ClusterPos {
            x: cluster.x,
            y: cluster.y - 1,
        };
        borders.push((above, Side::Down));
    }
    borders
}

/// Whether the border lies inside the grid — far-edge clusters have no right
/// or bottom neighbor.
fn border_exists(grid: &NavGrid, cluster_size: u32, (cluster, side): BorderKey) -> bool {
    match side {
        Side::Right => (cluster.x + 1) * cluster_size < grid.width(),
        Side::Down => (cluster.y + 1) * cluster_size < grid.height(),
    }
}

/// The cell pairs facing each other across one border, along it.
fn border_pairs(grid: &NavGrid, cluster_size: u32, (cluster, side): BorderKey) -> Vec<Transition> {
    match side {
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
    }
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
