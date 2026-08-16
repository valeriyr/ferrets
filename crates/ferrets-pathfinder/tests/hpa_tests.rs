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
    mover_shape::MoverShape,
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
        utils::nav(3, 3),
        utils::on_ground(),
        PlanTarget {
            cell: utils::nav(4, 4),
            size: CellSize::new(1, 1),
            stop: 1,
        },
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
        utils::world(0, 1),
        utils::on_ground(),
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
        utils::nav(1, 2),
        utils::on_ground(),
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
        utils::nav(1, 2),
        utils::on_ground(),
        utils::nav(8, 2),
    )
    .unwrap();

    // A sealed choke is crossed straight through, not refused.
    assert_eq!(
        cells,
        route(&[(2, 2), (3, 2), (4, 2), (5, 2), (6, 2), (7, 2), (8, 2)])
    );
}

#[test]
fn wide_detour_pays_for_claims_under_whole_footprint() {
    // Claims parked on row 3 alone: no route anchor ever stands on them, but
    // a 2x2 footprint walking row 2 covers them — the penalty must read the
    // whole anchored footprint, or the wide mover shoves straight through
    // the park it was asked to route around.
    let mut grid = utils::grid(16, 8);
    for x in 5..7 {
        grid.set_claimed_by(utils::GROUND, utils::nav(x, 3), true);
    }
    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 8, &[wide]);

    let cells = hpa::detour(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::nav(1, 2),
        wide,
        utils::nav(8, 2),
    )
    .unwrap();

    assert_eq!(
        cells,
        route(&[(2, 2), (3, 3), (4, 4), (5, 4), (6, 4), (7, 3), (8, 2)])
    );
    // No step's footprint covers the parked claims.
    assert!(cells.iter().all(|&cell| {
        !(grid.is_claimed_by(utils::GROUND, cell)
            || grid.is_claimed_by(utils::GROUND, CellPos::new(cell.x + 1, cell.y))
            || grid.is_claimed_by(utils::GROUND, CellPos::new(cell.x, cell.y + 1))
            || grid.is_claimed_by(utils::GROUND, CellPos::new(cell.x + 1, cell.y + 1)))
    }));
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
        utils::nav(0, 0),
        utils::on_ground(),
        PlanTarget {
            cell: utils::nav(6, 4),
            size: CellSize::new(1, 1),
            stop: 0,
        },
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
        utils::nav(0, 1),
        utils::on_ground(),
        PlanTarget {
            cell: utils::nav(15, 1),
            size: CellSize::new(1, 1),
            stop: 0,
        },
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
        from,
        utils::on_ground(),
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
            utils::nav(0, 1),
            utils::on_ground(),
            PlanTarget {
                cell: utils::nav(15, 1),
                size: CellSize::new(1, 1),
                stop: 0,
            },
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
// ─── Wide movers ──────────────────────────────────────────────────────────────
//

#[test]
fn wide_route_threads_gap_it_fits() {
    // A wall down x=7 with a two-cell hole at y=3..=4 — exactly the wide
    // body's height, so the only crossing anchors are the ones whose whole
    // footprint threads the hole.
    let mut grid = utils::grid(16, 8);
    for y in 0..8 {
        if y != 3 && y != 4 {
            grid.set_occupied(utils::GROUND, utils::nav(7, y), true);
        }
    }
    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 4, &[wide]);

    let cells = walk_as(
        &grid,
        &hierarchy,
        wide,
        utils::nav(0, 2),
        utils::nav(13, 2),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();

    // The anchors dip to y=3 for the crossing — footprint rows 3..=4, exactly
    // the hole — and climb back after it.
    assert_eq!(
        cells,
        route(&[
            (1, 2),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 3),
            (6, 3),
            (7, 3),
            (8, 3),
            (9, 3),
            (10, 2),
            (11, 2),
            (12, 2),
            (13, 2),
        ])
    );
}

#[test]
fn wide_goal_repairs_where_only_narrow_fits() {
    // A ring around the goal with a single-cell breach: a single-cell mover
    // slips in, but the wide body's region ends outside — its goal repairs
    // to the shore while the narrow one's does not.
    let mut grid = utils::hollow_ring_grid(12, utils::nav(6, 4), 2);
    grid.set_occupied(utils::GROUND, utils::nav(6, 2), false);
    let narrow = utils::on_ground();
    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 4, &[narrow, wide]);

    let narrow_cells = walk(
        &grid,
        &hierarchy,
        utils::nav(6, 0),
        utils::nav(6, 4),
        CellSize::new(1, 1),
        0,
    )
    .unwrap();
    assert_eq!(narrow_cells.last(), Some(&utils::nav(6, 4)));

    let path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::nav(6, 0),
        wide,
        PlanTarget {
            cell: utils::nav(6, 4),
            size: CellSize::new(1, 1),
            stop: 0,
        },
        hpa::PathShape::CellSteps,
    )
    .unwrap();

    // Repaired: the nearest anchor in the wide body's own region, on the
    // shore instead of through the one-cell breach. Several shore anchors
    // tie on the footprint's edge distance; the tie breaks by cell order
    // across every ring that can still carry the tying rank — the scan may
    // not stop at the first ring that produced it.
    assert_eq!(
        path.target,
        PlanTarget {
            cell: utils::nav(2, 0),
            size: CellSize::new(1, 1),
            stop: 0,
        }
    );
}

#[test]
fn ranged_goal_is_reached_by_edge_from_foreign_cluster() {
    // A full wall down x=8 seals the goal's side of the map from the start's;
    // the goal sits just past the wall at (9,2), stop 2. No route enters the
    // goal's own cluster — but an anchor at x=6 puts a 2x2 footprint's edge
    // exactly 2 from the goal, shooting over the wall. The corridor gathers
    // acceptance entries from every cluster that can hold an accepting
    // anchor, so the plan succeeds, the goal survives unrepaired, and the
    // walk ends on the near side of the wall.
    let mut grid = utils::grid(16, 8);
    for y in 0..8 {
        grid.set_occupied(utils::GROUND, utils::nav(8, y), true);
    }
    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 4, &[wide]);

    let path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::nav(0, 2),
        wide,
        PlanTarget {
            cell: utils::nav(9, 2),
            size: CellSize::ONE,
            stop: 2,
        },
        hpa::PathShape::CellSteps,
    )
    .expect("acceptance exists west of the wall");
    assert_eq!(
        path.target,
        PlanTarget {
            cell: utils::nav(9, 2),
            size: CellSize::ONE,
            stop: 2,
        },
        "an edge-reachable goal was repaired away"
    );

    let cells = walk_as(
        &grid,
        &hierarchy,
        wide,
        utils::nav(0, 2),
        utils::nav(9, 2),
        CellSize::ONE,
        2,
    )
    .unwrap();
    // The exact stand: anchor (6,2) puts the footprint's edge at x=7, two
    // cells from the goal — the walk never crosses the wall.
    assert_eq!(cells.last(), Some(&utils::nav(6, 2)));
}

#[test]
fn refine_fails_when_crossing_no_longer_fits_wide_shape() {
    let mut grid = utils::grid(16, 4);
    let wide = utils::square_on_ground(2);
    let mut hierarchy = utils::build_for(&grid, 4, &[wide]);

    let mut path = hpa::find_path(
        &grid,
        &hierarchy,
        PROJECTION,
        utils::nav(0, 1),
        wide,
        PlanTarget {
            cell: utils::nav(13, 1),
            size: CellSize::new(1, 1),
            stop: 0,
        },
        hpa::PathShape::CellSteps,
    )
    .unwrap();
    assert!(!path.corridor.is_empty());

    // Block a cell the crossing's own anchor does not stand on but its
    // footprint covers: a single-cell mover would still pass, the wide one
    // must not.
    let next = *path.corridor.last().unwrap();
    let blocked = utils::nav(next.to.x + 1, next.to.y);
    grid.set_occupied(utils::GROUND, blocked, true);
    assert!(grid.is_statically_passable_by(utils::GROUND, next.to));
    hierarchy.mark_dirty(blocked);
    hierarchy.refresh(&grid);

    let from = *path.segment.last().unwrap();
    let segment = hpa::refine(
        &grid,
        &hierarchy,
        PROJECTION,
        from,
        wide,
        &mut path.corridor,
        path.target,
        hpa::PathShape::CellSteps,
    );
    assert_eq!(
        segment, None,
        "a crossing the footprint no longer fits must force a re-plan"
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
            utils::nav(0, 0),
            utils::on_ground(),
            PlanTarget {
                cell: utils::nav(31, 31),
                size: CellSize::new(1, 1),
                stop: 0,
            },
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
    walk_as(
        grid,
        hierarchy,
        utils::on_ground(),
        start,
        goal,
        goal_size,
        stop_distance,
    )
}

fn walk_as(
    grid: &NavGrid,
    hierarchy: &NavHierarchy,
    shape: MoverShape,
    start: CellPos,
    goal: CellPos,
    goal_size: CellSize,
    stop_distance: u32,
) -> Option<Vec<CellPos>> {
    let mut path = hpa::find_path(
        grid,
        hierarchy,
        PROJECTION,
        start,
        shape,
        PlanTarget {
            cell: goal,
            size: goal_size,
            stop: stop_distance,
        },
        hpa::PathShape::CellSteps,
    )?;

    let mut cells = path.segment.clone();
    // Arrival is judged the way the engine judges it: the anchor against the
    // shape's accepted goal, so a wide walk ends at its edge reach.
    let accepted =
        CellRect::new(path.target.cell, path.target.size).accepted_by(shape.size, path.target.stop);
    for _ in 0..1000 {
        let at = *cells.last().unwrap_or(&start);
        if PROJECTION.in_range_of_rect(at, accepted, path.target.stop) {
            validate_route(grid, shape, start, &cells);
            return Some(cells);
        }
        let segment = hpa::refine(
            grid,
            hierarchy,
            PROJECTION,
            at,
            shape,
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
fn validate_route(grid: &NavGrid, shape: MoverShape, start: CellPos, cells: &[CellPos]) {
    let mut previous = start;
    for &cell in cells {
        assert!(
            previous.x.abs_diff(cell.x) <= 1 && previous.y.abs_diff(cell.y) <= 1,
            "route jumps from {previous:?} to {cell:?}"
        );
        assert!(
            grid.fits_statically(cell, shape),
            "route anchors the footprint on blocked ground at {cell:?}"
        );
        previous = cell;
    }
}
