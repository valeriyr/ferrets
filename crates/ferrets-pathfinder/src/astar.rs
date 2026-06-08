//! Finds the shortest passable path between two positions, with guaranteed determinism.

use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::layer_mask::LayerMask;

use super::{nav_grid::NavGrid, nav_pos::NavPos};

/// Defines movement costs and range metrics for the map type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Isometric projection — all 8 directions cost equally and appear the same distance on screen.
    Isometric,
    /// Orthogonal top-down — diagonal moves cover √2 more ground and cost more.
    Orthogonal,
}

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

/// Movement cost for a cardinal (non-diagonal) step.
const CARDINAL_COST: u32 = 10;
/// Movement cost for a diagonal step — approximates √2 × [`CARDINAL_COST`].
const DIAGONAL_COST: u32 = 14;

/// Finds the shortest path for a unit with `layer_mask` from `start` toward `goal`.
///
/// `stop_distance` controls how close the path must get to `goal` — `0` requires
/// reaching the exact position. The distance metric depends on the projection:
/// Chebyshev tiles for `Isometric`, Euclidean tiles for `Orthogonal`.
///
/// Returns the sequence of positions to visit (excluding `start`), or `None` if no path exists.
pub fn find_path(
    grid: &NavGrid,
    projection: Projection,
    layer_mask: impl Into<LayerMask>,
    start: FixedUVec2,
    goal: FixedUVec2,
    stop_distance: u32,
) -> Option<Vec<FixedUVec2>> {
    let layer_mask = layer_mask.into();

    let start = NavPos::from(start);
    let goal = NavPos::from(goal);

    if stop_distance == 0 && start != goal && grid.is_occupied_by(layer_mask, goal) {
        return None;
    }

    if in_range(projection, start, goal, stop_distance) {
        return Some(vec![]);
    }

    let result = pathfinding::prelude::astar(
        &start,
        |&pos| {
            DIRECTIONS.iter().filter_map(move |&(dx, dy)| {
                let nx = pos.x as i32 + dx;
                let ny = pos.y as i32 + dy;
                if nx < 0 || ny < 0 {
                    return None;
                }
                let neighbor = NavPos::new(nx as u32, ny as u32);
                if grid.is_occupied_by(layer_mask, neighbor) {
                    return None;
                }
                let is_diagonal = dx != 0 && dy != 0;
                if is_diagonal && !allows_diagonal(grid, layer_mask, pos, neighbor) {
                    return None;
                }
                let cost = step_cost(projection, is_diagonal);
                Some((neighbor, cost))
            })
        },
        |&pos| heuristic(projection, pos, goal, stop_distance),
        |pos| in_range(projection, *pos, goal, stop_distance),
    );

    result.map(|(path, _cost)| path.into_iter().skip(1).map(|p| p.into()).collect())
}

/// Returns `true` when both cardinal positions adjacent to a diagonal step are
/// passable, preventing movement through the gap between two diagonal walls.
fn allows_diagonal(grid: &NavGrid, mask: LayerMask, from: NavPos, to: NavPos) -> bool {
    grid.is_passable_by(mask, NavPos::new(to.x, from.y))
        && grid.is_passable_by(mask, NavPos::new(from.x, to.y))
}

/// Movement cost for a single step given the map projection.
fn step_cost(projection: Projection, is_diagonal: bool) -> u32 {
    match projection {
        Projection::Isometric => CARDINAL_COST,
        Projection::Orthogonal => {
            if is_diagonal {
                DIAGONAL_COST
            } else {
                CARDINAL_COST
            }
        }
    }
}

/// Returns `true` if `from` is within `stop_distance` of `to`.
fn in_range(projection: Projection, from: NavPos, to: NavPos, stop_distance: u32) -> bool {
    match projection {
        Projection::Isometric => chebyshev(from, to) <= stop_distance,
        Projection::Orthogonal => {
            let dx = from.x.abs_diff(to.x);
            let dy = from.y.abs_diff(to.y);

            dx * dx + dy * dy <= stop_distance * stop_distance
        }
    }
}

/// A* cost estimate from `from` toward `to`, adjusted for `stop_distance`.
fn heuristic(projection: Projection, from: NavPos, to: NavPos, stop_distance: u32) -> u32 {
    match projection {
        Projection::Isometric => chebyshev(from, to).saturating_sub(stop_distance) * CARDINAL_COST,
        Projection::Orthogonal => octile(from, to).saturating_sub(stop_distance * DIAGONAL_COST),
    }
}

/// Chebyshev distance — maximum of horizontal and vertical distances between two positions.
fn chebyshev(a: NavPos, b: NavPos) -> u32 {
    a.x.abs_diff(b.x).max(a.y.abs_diff(b.y))
}

/// Octile distance — minimum movement cost accounting for diagonal and cardinal step costs.
fn octile(a: NavPos, b: NavPos) -> u32 {
    let dx = a.x.abs_diff(b.x);
    let dy = a.y.abs_diff(b.y);

    let diag = dx.min(dy);
    let straight = dx.max(dy) - diag;

    diag * DIAGONAL_COST + straight * CARDINAL_COST
}
