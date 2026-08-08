//! Hierarchical pathfinding: planning, goal repair, travel-time refinement,
//! and smoothing.
//!
//! Routes are asserted exactly: lockstep peers replay these searches
//! independently, so a changed route — however equivalent — is a desync,
//! not a style choice. Changing a pinned route must be a conscious decision.

mod utils;

use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_pathfinder::{
    astar,
    hierarchy::NavHierarchy,
    hpa::{self, PlanTarget},
    nav_grid::NavGrid,
};

//
// ─── Planning ─────────────────────────────────────────────────────────────────
//

#[test]
fn goal_in_range_yields_empty_route() {
    let grid = utils::grid(8, 8);
    let hierarchy = utils::build_ground(&grid, 4);

    let path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        utils::nav(3, 3),
        utils::nav(4, 4),
        CellSize::new(1, 1),
        1,
        hpa::PathShape::CellSteps,
    )
    .unwrap();

    assert!(path.segment.is_empty());
    assert!(path.corridor.is_empty());
    assert_eq!(
        path.target,
        PlanTarget {
            cell: utils::nav(4, 4),
            size: CellSize::new(1, 1),
            stop: 1,
        }
    );
}

#[test]
fn open_grid_route_is_optimal() {
    let grid = utils::grid(16, 4);
    let hierarchy = utils::build_ground(&grid, 4);

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 0),
        utils::nav(15, 3),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();

    // Fifteen steps is the Chebyshev distance — the hierarchical walk loses
    // nothing to the abstraction on open ground.
    assert_eq!(
        cells,
        route(&[
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 2),
            (6, 2),
            (7, 2),
            (8, 2),
            (9, 2),
            (10, 2),
            (11, 2),
            (12, 2),
            (13, 2),
            (14, 3),
            (15, 3),
        ])
    );
}

#[test]
fn route_threads_through_only_gap() {
    // A wall down x=7 with one gap forces every route through it.
    let mut grid = utils::grid(16, 8);
    for y in 0..8 {
        if y != 6 {
            grid.set_occupied(utils::GROUND, utils::nav(7, y), true);
        }
    }
    let hierarchy = utils::build_ground(&grid, 4);

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 0),
        utils::nav(15, 0),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();

    assert_eq!(
        cells,
        route(&[
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
            (5, 4),
            (6, 5),
            (6, 6),
            (7, 6),
            (8, 6),
            (9, 5),
            (9, 4),
            (10, 3),
            (11, 2),
            (12, 2),
            (13, 1),
            (14, 1),
            (15, 0),
        ])
    );
}

#[test]
fn long_route_ignores_unit_claims() {
    // A wall of unit claims blocks the flat search but not the plan; the
    // conflict is movement's to resolve at crossing time.
    let mut grid = utils::grid(16, 4);
    for y in 0..4 {
        grid.set_claimed_by(utils::GROUND, utils::nav(8, y), true);
    }
    let hierarchy = utils::build_ground(&grid, 4);

    let flat = astar::find_path(
        &grid,
        PROJECTION,
        utils::GROUND,
        utils::world(0, 1),
        utils::world(15, 1),
        CellSize::new(1, 1),
        0,
    );
    assert!(flat.is_none(), "the flat search must honor claims");

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 1),
        utils::nav(15, 1),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();
    // The route walks straight over the claimed column at (8, 2).
    assert_eq!(
        cells,
        route(&[
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 2),
            (6, 2),
            (7, 2),
            (8, 2),
            (9, 2),
            (10, 2),
            (11, 2),
            (12, 2),
            (13, 2),
            (14, 1),
            (15, 1),
        ])
    );
}

//
// ─── Detour ───────────────────────────────────────────────────────────────────
//
// A detour treats unit claims as a soft cost, not a wall: expensive enough
// to route around parked units, cheap enough that a fully claimed choke is
// still crossed.
//

#[test]
fn detour_routes_around_claims() {
    // A two-column block of claims across the rows, with the top row open:
    //
    //   . . . . . . . . . .   y=0   open bypass
    //   . . . . . X X . . .   y=1
    //   . F . . . X X . R .   y=2   F = from (1,2), R = rejoin (8,2)
    //   . . . . . X X . . .   y=3   X = claimed (5..6, 1..7)
    //           ...
    let mut grid = utils::grid(16, 8);
    for y in 1..8 {
        for x in 5..7 {
            grid.set_claimed_by(utils::GROUND, utils::nav(x, y), true);
        }
    }
    let hierarchy = utils::build_ground(&grid, 8);

    let cells = hpa::detour(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        utils::nav(1, 2),
        utils::nav(8, 2),
    )
    .unwrap();

    // Two claimed columns cost more than the open bypass over y = 0.
    assert_eq!(
        cells,
        route(&[(2, 2), (3, 2), (4, 1), (5, 0), (6, 0), (7, 1), (8, 2)])
    );
    assert!(
        cells
            .iter()
            .all(|&cell| !grid.is_claimed_by(utils::GROUND, cell))
    );
}

#[test]
fn detour_crosses_fully_claimed_choke() {
    // The same block sealed to the map edge: no unclaimed way remains, and
    // the detour crosses the claims rather than failing.
    let mut grid = utils::grid(16, 8);
    for y in 0..8 {
        for x in 5..7 {
            grid.set_claimed_by(utils::GROUND, utils::nav(x, y), true);
        }
    }
    let hierarchy = utils::build_ground(&grid, 8);

    let cells = hpa::detour(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        utils::nav(1, 2),
        utils::nav(8, 2),
    )
    .unwrap();

    // A sealed choke is crossed straight through, not refused.
    assert_eq!(
        cells,
        route(&[(2, 2), (3, 2), (4, 2), (5, 2), (6, 2), (7, 2), (8, 2)])
    );
}

//
// ─── Goal repair ──────────────────────────────────────────────────────────────
//

#[test]
fn goal_inside_walled_pond_repairs_to_shore() {
    // A closed ring walls off the pond around (6, 4); ordering a mover into
    // it walks to the nearest cell outside the ring instead of failing.
    let grid = utils::hollow_ring_grid(12, utils::nav(6, 4), 2);
    let hierarchy = utils::build_ground(&grid, 4);

    let path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        utils::nav(0, 0),
        utils::nav(6, 4),
        CellSize::new(1, 1),
        0,
        hpa::PathShape::CellSteps,
    )
    .unwrap();

    // The nearest shore cell outside the ring, by metric then cell order.
    assert_eq!(
        path.target,
        PlanTarget {
            cell: utils::nav(3, 1),
            size: CellSize::new(1, 1),
            stop: 0,
        }
    );

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 0),
        utils::nav(6, 4),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();
    assert_eq!(cells, route(&[(1, 0), (2, 1), (3, 1)]));
}

#[test]
fn occupied_goal_with_stop_distance_reaches_adjacent_cell() {
    // A statically occupied goal cell (a building) with stop distance 1:
    // the route ends on a neighboring cell without repair.
    let mut grid = utils::grid(16, 4);
    grid.set_occupied(utils::GROUND, utils::nav(14, 2), true);
    let hierarchy = utils::build_ground(&grid, 4);

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 2),
        utils::nav(14, 2),
        CellSize::new(1, 1),
        1,
    )
    .unwrap();

    // Straight down the row, stopping on the neighbor of the occupied goal.
    assert_eq!(
        cells,
        route(&[
            (1, 2),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 2),
            (6, 2),
            (7, 2),
            (8, 2),
            (9, 2),
            (10, 2),
            (11, 2),
            (12, 2),
            (13, 2),
        ])
    );
}

//
// ─── Refinement under change ──────────────────────────────────────────────────
//

#[test]
fn refine_fails_when_crossing_gets_walled() {
    let mut grid = utils::grid(16, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    let mut path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        utils::nav(0, 1),
        utils::nav(15, 1),
        CellSize::new(1, 1),
        0,
        hpa::PathShape::CellSteps,
    )
    .unwrap();
    assert!(!path.corridor.is_empty());

    // Wall the far side of the next crossing after the plan was made.
    let next = *path.corridor.last().unwrap();
    grid.set_occupied(utils::GROUND, next.to, true);
    hierarchy.mark_dirty(next.to);
    hierarchy.refresh(&grid);

    let from = *path.segment.last().unwrap();
    let segment = hpa::refine(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::GROUND,
        from,
        &mut path.corridor,
        path.target,
        hpa::PathShape::CellSteps,
    );
    assert_eq!(segment, None, "a stale crossing must force a re-plan");
}

#[test]
fn replanning_after_refresh_avoids_new_wall() {
    let mut grid = utils::grid(16, 8);
    let mut hierarchy = utils::build_ground(&grid, 4);

    walk(
        &grid,
        &hierarchy,
        utils::nav(0, 1),
        utils::nav(15, 1),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();

    // A wall with one gap at the bottom lands mid-game; the warm cost cache
    // must not leak the old open routes.
    for y in 0..7 {
        grid.set_occupied(utils::GROUND, utils::nav(8, y), true);
        hierarchy.mark_dirty(utils::nav(8, y));
    }
    hierarchy.refresh(&grid);

    // The refreshed warm hierarchy must plan exactly what a cold one built
    // on the mutated grid plans — eviction may not leak the old routes.
    let plan = |hierarchy: &NavHierarchy| {
        hpa::find_path(
            &grid,
            hierarchy,
            PROJECTION,
            utils::GROUND,
            utils::nav(0, 1),
            utils::nav(15, 1),
            CellSize::new(1, 1),
            0,
            hpa::PathShape::CellSteps,
        )
        .unwrap()
    };
    assert_eq!(plan(&hierarchy), plan(&utils::build_ground(&grid, 4)));

    let cells = walk(
        &grid,
        &hierarchy,
        utils::nav(0, 1),
        utils::nav(15, 1),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();
    // The gap at (8, 7) is the only way through the new wall.
    assert_eq!(
        cells,
        route(&[
            (1, 2),
            (1, 3),
            (2, 4),
            (3, 5),
            (4, 6),
            (5, 6),
            (6, 7),
            (7, 7),
            (8, 7),
            (9, 7),
            (10, 6),
            (11, 6),
            (12, 6),
            (13, 5),
            (13, 4),
            (14, 3),
            (15, 2),
            (15, 1),
        ])
    );
}

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

#[test]
fn planning_is_identical_on_cold_and_warm_caches() {
    let grid = utils::hollow_ring_grid(32, utils::nav(16, 16), 6);
    let hierarchy = utils::build_ground(&grid, 8);

    let plan = |hierarchy: &NavHierarchy| {
        hpa::find_path(
            &grid,
            hierarchy,
            PROJECTION,
            utils::GROUND,
            utils::nav(0, 0),
            utils::nav(31, 31),
            CellSize::new(1, 1),
            0,
            hpa::PathShape::CellSteps,
        )
        .unwrap()
    };

    let cold = plan(&utils::build_ground(&grid, 8));
    let warm_once = plan(&hierarchy);
    let warm_twice = plan(&hierarchy);

    assert_eq!(cold, warm_once);
    assert_eq!(warm_once, warm_twice);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

const PROJECTION: Projection = Projection::Isometric;

/// Plans and walks the whole route the way movement will: consume the first
/// segment, refine the corridor crossing by crossing, then the final leg to
/// the target. Returns every cell walked, validated to be a contiguous,
/// statically passable route ending in range of the (possibly repaired)
/// target.
fn walk(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<Vec<CellPos>> {
    let mut path = hpa::find_path(
        grid,
        hierarchy,
        PROJECTION,
        utils::GROUND,
        start,
        goal,
        goal_size,
        stop_distance,
        hpa::PathShape::CellSteps,
    )?;

    let mut cells = path.segment.clone();
    for _ in 0..1000 {
        let at = *cells.last().unwrap_or(&start);
        if PROJECTION.in_range_of_rect(
            at,
            CellRect::new(path.target.cell, path.target.size),
            path.target.stop,
        ) {
            validate_route(grid, start, &cells);
            return Some(cells);
        }
        let segment = hpa::refine(
            grid,
            hierarchy,
            PROJECTION,
            utils::GROUND,
            at,
            &mut path.corridor,
            path.target,
            hpa::PathShape::CellSteps,
        )?;
        assert!(!segment.is_empty(), "an unfinished route must keep moving");
        cells.extend(segment);
    }
    panic!("route did not converge");
}

/// A route literal, cell by cell.
fn route(cells: &[(u32, u32)]) -> Vec<CellPos> {
    cells.iter().map(|&(x, y)| utils::nav(x, y)).collect()
}

/// Asserts the route is contiguous (every step between adjacent cells) and
/// statically passable.
fn validate_route(grid: &NavGrid, start: CellPos, cells: &[CellPos]) {
    let mut previous = start;
    for &cell in cells {
        assert!(
            previous.x.abs_diff(cell.x) <= 1 && previous.y.abs_diff(cell.y) <= 1,
            "route jumps from {previous:?} to {cell:?}"
        );
        assert!(
            grid.is_statically_passable_by(utils::GROUND, cell),
            "route walks through blocked {cell:?}"
        );
        previous = cell;
    }
}
