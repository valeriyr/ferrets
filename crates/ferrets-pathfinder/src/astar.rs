//! Finds the shortest passable path between two positions, with guaranteed determinism.

use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection, Step},
};
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{layer_mask::LayerMask, nav_grid::NavGrid};

/// Movement directions: (dx, dy). Costs depend on [`Projection`].
const DIRECTIONS: [(i32, i32); 8] = [
    (0, -1),
    (0, 1),
    (-1, 0),
    (1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// Which blockers a navigation query honors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blockers {
    /// Static occupancy and unit claims alike.
    All,
    /// Static occupancy only — unit claims are ignored.
    Static,
}

impl Blockers {
    /// Returns `true` when `pos` is passable for `mask` under this blocker
    /// mode.
    pub(crate) fn passable(self, grid: &NavGrid, mask: LayerMask, pos: CellPos) -> bool {
        match self {
            Blockers::All => grid.is_passable_by(mask, pos),
            Blockers::Static => grid.is_statically_passable_by(mask, pos),
        }
    }
}

/// Finds the shortest path for a unit with `layer_mask` from `start` toward the
/// footprint covering `goal_size` cells from `goal`.
///
/// `stop_distance` controls how close the path must get to the footprint — `0`
/// requires standing on one of its cells. The distance metric depends on the
/// projection: Chebyshev cells for `Isometric`, Euclidean cells for `Orthogonal`.
///
/// Returns the sequence of positions to visit (excluding `start`), or `None` if no path exists.
pub fn find_path(
    grid: &NavGrid,
    projection: Projection,
    layer_mask: impl Into<LayerMask>,
    start: FixedUVec2,
    goal: FixedUVec2,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<Vec<FixedUVec2>> {
    let layer_mask = layer_mask.into();

    let start = CellPos::from(start);
    let goal = CellPos::from(goal);

    if projection.in_range_of_rect(start, CellRect::new(goal, goal_size), stop_distance) {
        return Some(vec![]);
    }

    // Stopping at no distance at all means standing somewhere on the destination
    // footprint, which cannot happen with every cell of it occupied.
    if stop_distance == 0
        && CellRect::new(goal, goal_size)
            .cells()
            .all(|cell| grid.is_occupied_by(layer_mask, cell))
    {
        return None;
    }

    let result = pathfinding::prelude::astar(
        &start,
        |&pos| {
            passable_neighbors(grid, layer_mask, pos, Blockers::All)
                .map(|(neighbor, step)| (neighbor, projection.step_cost(step)))
        },
        |&pos| {
            heuristic(
                projection,
                pos,
                CellRect::new(goal, goal_size),
                stop_distance,
            )
        },
        |pos| projection.in_range_of_rect(*pos, CellRect::new(goal, goal_size), stop_distance),
    );

    result.map(|(path, _cost)| path.into_iter().skip(1).map(|p| p.into()).collect())
}

/// The passable neighbors of `pos` for `mask`, in fixed direction order, each
/// with its step class, with the corner rule applied — the one connectivity
/// definition every search and flood fill must share.
pub(crate) fn passable_neighbors(
    grid: &NavGrid,
    mask: LayerMask,
    pos: CellPos,
    blockers: Blockers,
) -> impl Iterator<Item = (CellPos, Step)> {
    DIRECTIONS.iter().filter_map(move |&(dx, dy)| {
        let nx = pos.x as i32 + dx;
        let ny = pos.y as i32 + dy;
        if nx < 0 || ny < 0 {
            return None;
        }
        let neighbor = CellPos::new(nx as u32, ny as u32);
        if !blockers.passable(grid, mask, neighbor) {
            return None;
        }
        let step = if dx != 0 && dy != 0 {
            Step::Diagonal
        } else {
            Step::Cardinal
        };
        match step {
            Step::Diagonal => {
                if !allows_diagonal(grid, mask, pos, neighbor, blockers) {
                    return None;
                }
            }
            Step::Cardinal => {}
        }
        Some((neighbor, step))
    })
}

/// Returns `true` when both cardinal positions adjacent to a diagonal step are
/// passable, preventing movement through the gap between two diagonal walls.
pub(crate) fn allows_diagonal(
    grid: &NavGrid,
    mask: LayerMask,
    from: CellPos,
    to: CellPos,
    blockers: Blockers,
) -> bool {
    blockers.passable(grid, mask, CellPos::new(to.x, from.y))
        && blockers.passable(grid, mask, CellPos::new(from.x, to.y))
}

/// Finds the shortest path confined to the `window` rect — cells outside it
/// count as impassable — under the given blocker mode, with the same goal
/// semantics as [`find_path`]. Returns the positions to visit (excluding
/// `start`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bounded_path(
    grid: &NavGrid,
    projection: Projection,
    mask: LayerMask,
    blockers: Blockers,
    window: CellRect,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
    extra_cost: impl Fn(CellPos) -> u32,
) -> Option<Vec<CellPos>> {
    bounded_search(
        grid,
        projection,
        mask,
        blockers,
        window,
        start,
        goal,
        goal_size,
        stop_distance,
        extra_cost,
    )
    .map(|(path, _)| path)
}

/// The cost of the cheapest path from `from` to exactly `to` confined to the
/// `window` rect, or `None` when the window does not connect them.
pub(crate) fn bounded_cost(
    grid: &NavGrid,
    projection: Projection,
    mask: LayerMask,
    blockers: Blockers,
    window: CellRect,
    from: CellPos,
    to: CellPos,
) -> Option<u32> {
    bounded_search(
        grid,
        projection,
        mask,
        blockers,
        window,
        from,
        to,
        CellSize::new(1, 1),
        0,
        |_| 0,
    )
    .map(|(_, cost)| cost)
}

/// The window-confined search behind [`bounded_path`] and [`bounded_cost`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn bounded_search(
    grid: &NavGrid,
    projection: Projection,
    mask: LayerMask,
    blockers: Blockers,
    window: CellRect,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
    extra_cost: impl Fn(CellPos) -> u32,
) -> Option<(Vec<CellPos>, u32)> {
    if projection.in_range_of_rect(start, CellRect::new(goal, goal_size), stop_distance) {
        return Some((vec![], 0));
    }

    let in_window = move |pos: CellPos| window.contains(pos);

    let result = pathfinding::prelude::astar(
        &start,
        |&pos| {
            passable_neighbors(grid, mask, pos, blockers)
                .filter(move |&(neighbor, _)| in_window(neighbor))
                .map(|(neighbor, step)| {
                    (neighbor, projection.step_cost(step) + extra_cost(neighbor))
                })
        },
        |&pos| {
            heuristic(
                projection,
                pos,
                CellRect::new(goal, goal_size),
                stop_distance,
            )
        },
        |pos| projection.in_range_of_rect(*pos, CellRect::new(goal, goal_size), stop_distance),
    );

    result.map(|(path, cost)| (path.into_iter().skip(1).collect(), cost))
}

/// A one-unit cost nudge that separates opposing flows into lanes:
/// rightward travel prefers even rows, leftward odd ones, and likewise for
/// vertical travel over columns.
pub(crate) fn lane_bias(from: CellPos, toward: CellPos) -> impl Fn(CellPos) -> u32 {
    let rightward = toward.x >= from.x;
    let downward = toward.y >= from.y;
    move |cell: CellPos| {
        let row = if rightward {
            cell.y % 2
        } else {
            (cell.y + 1) % 2
        };
        let column = if downward {
            cell.x % 2
        } else {
            (cell.x + 1) % 2
        };
        row + column
    }
}

/// A* cost estimate from `from` toward the rectangle at `goal`/`goal_size`, adjusted
/// for `stop_distance`.
///
/// Measured to the nearest cell of the rectangle, which is what the goal test
/// accepts — estimating against a far corner instead would overshoot and could send
/// the search the long way round.
fn heuristic(projection: Projection, from: CellPos, goal: CellRect, stop_distance: u32) -> u32 {
    let nearest = from.clamp_to_rect(goal);
    match projection {
        // The stop distance is credited at the LARGEST possible per-cell step
        // cost, so the estimate can only err downward — a smaller credit
        // would overestimate the remaining cost for some approach direction
        // and break the search's optimality.
        Projection::Isometric => {
            projection::chebyshev(from, nearest).saturating_sub(stop_distance)
                * projection.step_cost(Step::Cardinal)
        }
        Projection::Orthogonal => projection::octile(from, nearest)
            .saturating_sub(stop_distance * projection.step_cost(Step::Diagonal)),
    }
}
