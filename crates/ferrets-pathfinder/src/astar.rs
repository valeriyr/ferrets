//! Finds the shortest passable path between two positions, with guaranteed determinism.

use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection, Step},
};
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{
    mover_profile::{Blockers, MoverProfile},
    mover_shape::MoverShape,
    nav_grid::NavGrid,
};

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

/// Finds the shortest path for `shape` from `start` toward the footprint
/// covering `goal_size` cells from `goal`.
///
/// The path is a sequence of **anchor** cells: each is a position the shape's
/// whole footprint fits at, so a wide mover never routes through a gap it
/// cannot pass.
///
/// `stop_distance` controls how close the path must get to the footprint — `0`
/// requires standing on one of its cells. The distance metric depends on the
/// projection: Chebyshev cells for `Isometric`, Euclidean cells for `Orthogonal`.
///
/// Returns the sequence of positions to visit (excluding `start`), or `None` if no path exists.
pub fn find_path(
    grid: &NavGrid,
    projection: Projection,
    start: FixedUVec2,
    shape: MoverShape,
    goal: FixedUVec2,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<Vec<FixedUVec2>> {
    let profile = MoverProfile::new(shape, Blockers::All);

    let start = CellPos::from(start);
    let goal = CellPos::from(goal);

    let accepted = CellRect::new(goal, goal_size).accepted_by(shape.size, stop_distance);
    if projection.in_range_of_rect(start, accepted, stop_distance) {
        return Some(vec![]);
    }

    // Stopping at no distance at all means standing somewhere on the destination
    // footprint, which cannot happen with every cell of it occupied.
    if stop_distance == 0
        && CellRect::new(goal, goal_size)
            .cells()
            .all(|cell| grid.is_occupied_by(shape.mask, cell))
    {
        return None;
    }

    let result = pathfinding::prelude::astar(
        &start,
        |&pos| {
            passable_neighbors(grid, pos, profile)
                .map(|(neighbor, step)| (neighbor, projection.step_cost(step)))
        },
        |&pos| heuristic(projection, pos, accepted, stop_distance),
        |pos| projection.in_range_of_rect(*pos, accepted, stop_distance),
    );

    result.map(|(path, _cost)| path.into_iter().skip(1).map(|p| p.into()).collect())
}

/// The passable neighbors of `pos` for `profile`, in fixed direction order,
/// each with its step class, with the corner rule applied — the one
/// connectivity definition every search and flood fill must share.
pub(crate) fn passable_neighbors(
    grid: &NavGrid,
    pos: CellPos,
    profile: MoverProfile,
) -> impl Iterator<Item = (CellPos, Step)> {
    DIRECTIONS.iter().filter_map(move |&(dx, dy)| {
        let nx = pos.x as i32 + dx;
        let ny = pos.y as i32 + dy;
        if nx < 0 || ny < 0 {
            return None;
        }
        let neighbor = CellPos::new(nx as u32, ny as u32);
        if !grid.fits_for(neighbor, profile) {
            return None;
        }
        let step = if dx != 0 && dy != 0 {
            Step::Diagonal
        } else {
            Step::Cardinal
        };
        match step {
            Step::Diagonal => {
                if !allows_diagonal(grid, pos, profile, neighbor) {
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
    from: CellPos,
    profile: MoverProfile,
    to: CellPos,
) -> bool {
    grid.fits_for(CellPos::new(to.x, from.y), profile)
        && grid.fits_for(CellPos::new(from.x, to.y), profile)
}

/// Finds the shortest path confined to the `window` rect — cells outside it
/// count as impassable — under the given blocker mode, with the same goal
/// semantics as [`find_path`]. Returns the positions to visit (excluding
/// `start`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bounded_path(
    grid: &NavGrid,
    projection: Projection,
    window: CellRect,
    start: CellPos,
    profile: MoverProfile,
    goal: CellRect,
    stop_distance: u32,
    extra_cost: impl Fn(CellPos) -> u32,
) -> Option<Vec<CellPos>> {
    bounded_search(
        grid,
        projection,
        window,
        start,
        profile,
        goal,
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
    window: CellRect,
    from: CellPos,
    profile: MoverProfile,
    to: CellPos,
) -> Option<u32> {
    bounded_search(
        grid,
        projection,
        window,
        from,
        profile,
        CellRect::cell(to),
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
    window: CellRect,
    start: CellPos,
    profile: MoverProfile,
    goal: CellRect,
    stop_distance: u32,
    extra_cost: impl Fn(CellPos) -> u32,
) -> Option<(Vec<CellPos>, u32)> {
    let accepted = goal.accepted_by(profile.shape.size, stop_distance);
    if projection.in_range_of_rect(start, accepted, stop_distance) {
        return Some((vec![], 0));
    }

    let in_window = move |pos: CellPos| window.contains(pos);

    let result = pathfinding::prelude::astar(
        &start,
        |&pos| {
            passable_neighbors(grid, pos, profile)
                .filter(move |&(neighbor, _)| in_window(neighbor))
                .map(|(neighbor, step)| {
                    (neighbor, projection.step_cost(step) + extra_cost(neighbor))
                })
        },
        |&pos| heuristic(projection, pos, accepted, stop_distance),
        |pos| projection.in_range_of_rect(*pos, accepted, stop_distance),
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

/// A* cost estimate from `from` toward the accepted `goal` rectangle —
/// already grown for the searcher's footprint — adjusted for `stop_distance`.
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
