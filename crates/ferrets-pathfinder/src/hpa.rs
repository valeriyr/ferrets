//! Hierarchical pathfinding: long-range planning over the cluster
//! abstraction, refined into walkable cells segment by segment as the mover
//! travels.
//!
//! Long paths honor the static plane only — unit conflicts are a
//! movement-time concern. Unreachable goals are repaired to the nearest
//! reachable cell instead of failing: a mover ordered into a walled pond
//! walks to the shore.

use std::collections::BTreeMap;

use crate::{
    astar::{self, Blockers},
    hierarchy::NavHierarchy,
    layer_mask::LayerMask,
    nav_grid::NavGrid,
};
use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection},
};

/// One border crossing of a planned corridor, in travel orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Crossing {
    /// The side crossed from, in the cluster the mover comes through.
    pub from: CellPos,
    /// The side crossed onto, in the next cluster.
    pub to: CellPos,
}

/// The destination a plan leads to: the requested goal, or its nearest
/// reachable repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanTarget {
    /// The cell the plan walks toward.
    pub cell: CellPos,
    /// The goal footprint the final segment must reach — 1×1 for a repaired
    /// target.
    pub size: CellSize,
    /// The stop distance the final segment accepts — 0 for a repaired
    /// target.
    pub stop: u32,
}

/// The form a refined segment takes: one entry per cell for movement that
/// crosses cell by cell, or only the string-pulled corners for movement
/// that walks free positions — far waypoints keep a deflected mover pulled
/// through and past a blocker instead of into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathShape {
    CellSteps,
    Waypoints,
}

/// A planned long-range route: the cells to walk now, and the corridor still
/// to refine as travel proceeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchicalPath {
    /// The refined cells to walk now, in travel order, excluding the start.
    pub segment: Vec<CellPos>,
    /// The crossings still ahead, last first — pop from the back as each
    /// segment is consumed.
    pub corridor: Vec<Crossing>,
    /// The destination the corridor leads to.
    pub target: PlanTarget,
}

/// Plans a route from `start` toward the footprint covering `goal_size`
/// cells from `goal`, stopping within `stop_distance` of it, using the
/// hierarchy for the long range.
///
/// The returned path holds the refined first segment and the remaining
/// corridor; feed the corridor back through [`refine`] as segments are
/// consumed. Returns `None` only when `start` sits outside every region —
/// an unreachable goal is repaired, not failed.
///
/// Panics if no hierarchy abstraction was built for the mask.
#[allow(clippy::too_many_arguments)]
pub fn find_path(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: impl Into<LayerMask>,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
    shape: PathShape,
) -> Option<HierarchicalPath> {
    let mask = mask.into();

    if projection.in_range_of_rect(start, CellRect::new(goal, goal_size), stop_distance) {
        return Some(HierarchicalPath {
            segment: vec![],
            corridor: vec![],
            target: PlanTarget {
                cell: goal,
                size: goal_size,
                stop: stop_distance,
            },
        });
    }

    let (target, mut corridor) = plan_corridor(
        grid,
        hierarchy,
        projection,
        mask,
        start,
        goal,
        goal_size,
        stop_distance,
    )?;
    let segment = refine(
        grid,
        hierarchy,
        projection,
        mask,
        start,
        &mut corridor,
        target,
        shape,
    )?;

    Some(HierarchicalPath {
        segment,
        corridor,
        target,
    })
}

/// Plans only the corridor of a route: the (possibly repaired) target and
/// the crossings toward it, without refining any cells. [`refine`] turns it
/// into walkable segments — from any nearby start, which is what lets one
/// fanned group order share a single corridor.
///
/// Panics if no hierarchy abstraction was built for the mask.
#[allow(clippy::too_many_arguments)]
pub fn plan_corridor(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: impl Into<LayerMask>,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<(PlanTarget, Vec<Crossing>)> {
    let mask = mask.into();

    if projection.in_range_of_rect(start, CellRect::new(goal, goal_size), stop_distance) {
        return Some((
            PlanTarget {
                cell: goal,
                size: goal_size,
                stop: stop_distance,
            },
            vec![],
        ));
    }

    let start_region = hierarchy.region_of(mask, start)?;
    let target = repair_goal(
        grid,
        hierarchy,
        projection,
        mask,
        start_region,
        goal,
        goal_size,
        stop_distance,
    )?;

    // Adjacent clusters usually connect directly — an empty corridor lets
    // refinement path straight to the target; the abstract search covers a
    // detour around.
    if neighboring_clusters(hierarchy, start, target.cell) {
        let window = cluster_pair_window(hierarchy, start, target.cell);
        let direct = astar::bounded_path(
            grid,
            projection,
            mask,
            Blockers::Static,
            window,
            start,
            target.cell,
            target.size,
            target.stop,
            |_| 0,
        );
        if direct.is_some() {
            return Some((target, vec![]));
        }
    }

    let corridor = abstract_corridor(grid, hierarchy, projection, mask, start, target)?;
    Some((target, corridor))
}

/// Refines the next segment of a corridor: the cells from `from` to the far
/// side of the next crossing, or — with the corridor consumed — to the
/// target. Pops the crossing it consumes.
///
/// Returns `None` when the local window no longer connects the pieces (the
/// map changed under the plan); the caller re-plans from scratch.
#[allow(clippy::too_many_arguments)]
pub fn refine(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: impl Into<LayerMask>,
    from: CellPos,
    corridor: &mut Vec<Crossing>,
    target: PlanTarget,
    shape: PathShape,
) -> Option<Vec<CellPos>> {
    let mask = mask.into();

    let segment = match corridor.pop() {
        None => {
            let window = cluster_pair_window(hierarchy, from, target.cell);
            astar::bounded_path(
                grid,
                projection,
                mask,
                Blockers::Static,
                window,
                from,
                target.cell,
                target.size,
                target.stop,
                astar::lane_bias(from, target.cell),
            )?
        }
        Some(crossing) => {
            if !Blockers::Static.passable(grid, mask, crossing.to) {
                return None;
            }
            let window = cluster_pair_window(hierarchy, from, crossing.from);
            let mut segment = astar::bounded_path(
                grid,
                projection,
                mask,
                Blockers::Static,
                window,
                from,
                crossing.from,
                CellSize::new(1, 1),
                0,
                astar::lane_bias(from, crossing.from),
            )?;
            segment.push(crossing.to);
            segment
        }
    };

    Some(smooth(grid, mask, from, segment, shape))
}

/// What a detour pays for stepping onto a claimed cell instead of treating
/// it as a wall: expensive enough to flow around parked units, cheap enough
/// that a fully claimed choke is still crossed.
const CLAIM_PENALTY: u32 = 60;

/// Plans a short claim-aware detour from `from` back onto `rejoin`, confined
/// to the two cells' clusters. Unit claims cost [`CLAIM_PENALTY`] instead of
/// blocking, so the detour routes around parked units yet never fails on a
/// crowded choke; static occupancy still blocks.
pub fn detour(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: impl Into<LayerMask>,
    from: CellPos,
    rejoin: CellPos,
) -> Option<Vec<CellPos>> {
    let mask = mask.into();
    let window = cluster_pair_window(hierarchy, from, rejoin);
    let bias = astar::lane_bias(from, rejoin);
    astar::bounded_path(
        grid,
        projection,
        mask,
        Blockers::Static,
        window,
        from,
        rejoin,
        CellSize::new(1, 1),
        0,
        move |cell| {
            let claimed = if grid.is_claimed_by(mask, cell) {
                CLAIM_PENALTY
            } else {
                0
            };
            claimed + bias(cell)
        },
    )
}

/// Whether the two cells' clusters share a border or corner.
fn neighboring_clusters(hierarchy: &NavHierarchy, a: CellPos, b: CellPos) -> bool {
    hierarchy.cluster_of(a).touches(hierarchy.cluster_of(b))
}

/// The window covering both cells' clusters.
fn cluster_pair_window(hierarchy: &NavHierarchy, a: CellPos, b: CellPos) -> CellRect {
    hierarchy
        .cluster_rect(hierarchy.cluster_of(a))
        .union(hierarchy.cluster_rect(hierarchy.cluster_of(b)))
}

/// Keeps the goal when any cell accepting it connects to the start's region;
/// otherwise rewrites it to the nearest reachable cell (an exact 1×1 target).
#[allow(clippy::too_many_arguments)]
fn repair_goal(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: LayerMask,
    start_region: u32,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<PlanTarget> {
    // Every cell accepting the goal lies within the footprint's rectangle
    // grown by the stop distance.
    let min_x = goal.x.saturating_sub(stop_distance);
    let min_y = goal.y.saturating_sub(stop_distance);
    let max_x = (goal.x + goal_size.width + stop_distance).min(grid.width());
    let max_y = (goal.y + goal_size.height + stop_distance).min(grid.height());
    for y in min_y..max_y {
        for x in min_x..max_x {
            let cell = CellPos::new(x, y);
            if projection.in_range_of_rect(cell, CellRect::new(goal, goal_size), stop_distance)
                && hierarchy.region_of(mask, cell) == Some(start_region)
            {
                return Some(PlanTarget {
                    cell: goal,
                    size: goal_size,
                    stop: stop_distance,
                });
            }
        }
    }

    // Ring scan outward from the footprint for the nearest cell the start
    // connects to; ties break on metric first, cell order second. The rings
    // are Chebyshev-shaped scan structure, so the scan runs on past the
    // first hit while a later ring could still hold a closer cell — under
    // the orthogonal projection a cardinal cell of the next ring can beat a
    // diagonal one of this ring.
    let max_radius = grid.width().max(grid.height());
    let mut best: Option<(u32, CellPos)> = None;
    for radius in 1..=max_radius {
        if let Some((best_distance, _)) = best
            && projection.ring_floor(radius) >= best_distance
        {
            break;
        }
        for cell in ring_cells(grid, goal, goal_size, radius) {
            if hierarchy.region_of(mask, cell) != Some(start_region) {
                continue;
            }
            let distance = projection.rect_distance(cell, CellRect::new(goal, goal_size));
            let candidate = (distance, cell);
            if best.is_none_or(|current| candidate < current) {
                best = Some(candidate);
            }
        }
    }
    best.map(|(_, cell)| PlanTarget {
        cell,
        size: CellSize::new(1, 1),
        stop: 0,
    })
}

/// The in-bounds cells at Chebyshev distance exactly `radius` from the
/// rectangle at `origin`/`size`, in row-major order.
fn ring_cells(
    grid: &NavGrid,
    origin: CellPos,
    size: CellSize,
    radius: u32,
) -> impl Iterator<Item = CellPos> {
    let min_x = origin.x.saturating_sub(radius);
    let min_y = origin.y.saturating_sub(radius);
    let max_x = (origin.x + size.width + radius).min(grid.width());
    let max_y = (origin.y + size.height + radius).min(grid.height());
    let (width, height) = (grid.width(), grid.height());

    (min_y..max_y).flat_map(move |y| {
        (min_x..max_x).filter_map(move |x| {
            let cell = CellPos::new(x, y);
            if x >= width || y >= height {
                return None;
            }
            let on_ring =
                projection::chebyshev(cell, cell.clamp_to_rect(CellRect::new(origin, size)))
                    == radius;
            on_ring.then_some(cell)
        })
    })
}

/// A node of the abstract search: a transition-side cell, or the virtual
/// target every accepting cell links to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Node {
    Cell(CellPos),
    Target,
}

/// Plans the corridor of crossings over the abstract graph, last crossing
/// first. Returns `None` when no corridor connects start and target.
#[allow(clippy::too_many_arguments)]
fn abstract_corridor(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    projection: Projection,
    mask: LayerMask,
    start: CellPos,
    target: PlanTarget,
) -> Option<Vec<Crossing>> {
    let target_cluster = hierarchy.cluster_of(target.cell);

    // Entry costs into the virtual target from every transition side of its
    // cluster, under the target's own acceptance semantics.
    let target_window = hierarchy.cluster_rect(target_cluster);
    let mut target_costs: BTreeMap<CellPos, u32> = BTreeMap::new();
    for (side, _) in hierarchy.transition_sides(mask, target_cluster) {
        let cost = astar::bounded_search(
            grid,
            projection,
            mask,
            Blockers::Static,
            target_window,
            side,
            target.cell,
            target.size,
            target.stop,
            |_| 0,
        )
        .map(|(_, cost)| cost);
        if let Some(cost) = cost {
            target_costs.insert(side, cost);
        }
    }
    // The start's own cluster may be the target's.
    if hierarchy.cluster_of(start) == target_cluster
        && let Some((_, cost)) = astar::bounded_search(
            grid,
            projection,
            mask,
            Blockers::Static,
            target_window,
            start,
            target.cell,
            target.size,
            target.stop,
            |_| 0,
        )
    {
        target_costs.insert(start, cost);
    }

    let successors = |node: &Node| -> Vec<(Node, u32)> {
        let Node::Cell(cell) = *node else {
            return vec![];
        };
        let cluster = hierarchy.cluster_of(cell);
        let mut edges = Vec::new();

        for (side, transition) in hierarchy.transition_sides(mask, cluster) {
            if side == cell {
                // Cross the border: transition sides are cardinal-adjacent.
                let other = if transition.a == cell {
                    transition.b
                } else {
                    transition.a
                };
                edges.push((Node::Cell(other), projection.metric(cell, other)));
            } else if let Some(cost) =
                hierarchy.intra_cost(grid, projection, mask, cluster, cell, side)
            {
                edges.push((Node::Cell(side), cost));
            }
        }

        if cluster == target_cluster
            && let Some(&cost) = target_costs.get(&cell)
        {
            edges.push((Node::Target, cost));
        }

        edges
    };

    let result = pathfinding::prelude::astar(
        &Node::Cell(start),
        successors,
        |node| match node {
            Node::Cell(cell) => projection.metric(*cell, target.cell),
            Node::Target => 0,
        },
        |node| *node == Node::Target,
    );

    let (nodes, _) = result?;

    // Consecutive cells in different clusters are the corridor's crossings.
    let cells: Vec<CellPos> = nodes
        .into_iter()
        .filter_map(|node| match node {
            Node::Cell(cell) => Some(cell),
            Node::Target => None,
        })
        .collect();
    let mut corridor: Vec<Crossing> = cells
        .windows(2)
        .filter(|pair| hierarchy.cluster_of(pair[0]) != hierarchy.cluster_of(pair[1]))
        .map(|pair| Crossing {
            from: pair[0],
            to: pair[1],
        })
        .collect();
    corridor.reverse();
    Some(corridor)
}

/// String-pulls a refined segment, then renders it in the requested shape:
/// the pulled corners expanded back into adjacent cells so staircase
/// detours become straight runs, or the corners themselves.
fn smooth(
    grid: &NavGrid,
    mask: LayerMask,
    from: CellPos,
    segment: Vec<CellPos>,
    shape: PathShape,
) -> Vec<CellPos> {
    if segment.len() < 2 {
        return segment;
    }

    let mut result = Vec::with_capacity(segment.len());
    let mut anchor = from;
    let mut next = 0;
    while next < segment.len() {
        // The farthest waypoint the anchor sees in a straight line wins; the
        // immediate next cell is adjacent, so a step always exists.
        let mut advanced = false;
        for candidate in (next..segment.len()).rev() {
            if let Some(line) = line_of_cells(grid, mask, anchor, segment[candidate]) {
                anchor = segment[candidate];
                next = candidate + 1;
                match shape {
                    PathShape::CellSteps => result.extend(line),
                    PathShape::Waypoints => result.push(anchor),
                }
                advanced = true;
                break;
            }
        }
        if !advanced {
            // No straight line reaches even the adjacent cell (a claim-free
            // diagonal squeeze the search took): keep the plain step.
            result.push(segment[next]);
            anchor = segment[next];
            next += 1;
        }
    }
    result
}

/// The straight run of cells from `a` to `b` (excluding `a`), or `None` when
/// a cell on the way is statically blocked or a diagonal step cuts a corner.
fn line_of_cells(grid: &NavGrid, mask: LayerMask, a: CellPos, b: CellPos) -> Option<Vec<CellPos>> {
    let blockers = Blockers::Static;
    let dx = (b.x as i64 - a.x as i64).abs();
    let dy = -((b.y as i64 - a.y as i64).abs());
    let step_x: i64 = if a.x <= b.x { 1 } else { -1 };
    let step_y: i64 = if a.y <= b.y { 1 } else { -1 };
    let mut error = dx + dy;

    let mut cells = Vec::new();
    let (mut x, mut y) = (a.x as i64, a.y as i64);
    let mut current = a;
    while current != b {
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x += step_x;
        }
        if doubled <= dx {
            error += dx;
            y += step_y;
        }
        let next = CellPos::new(x as u32, y as u32);
        if !blockers.passable(grid, mask, next) {
            return None;
        }
        let is_diagonal = next.x != current.x && next.y != current.y;
        if is_diagonal && !astar::allows_diagonal(grid, mask, current, next, blockers) {
            return None;
        }
        cells.push(next);
        current = next;
    }
    Some(cells)
}
