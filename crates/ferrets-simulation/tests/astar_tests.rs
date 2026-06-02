mod utils;

use ferrets_simulation::pathfinding::{
    astar::{self, Projection},
    nav_pos::NavPos,
};

//
// ─── Isometric — exact goal ───────────────────────────────────────────────────
//

#[test]
fn straight_line_on_open_grid() {
    let grid = utils::grid(10, 10);
    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(3, 0),
        0,
    )
    .unwrap();

    assert_eq!(
        path,
        vec![utils::world(1, 0), utils::world(2, 0), utils::world(3, 0)]
    );
}

#[test]
fn isometric_routes_around_wall() {
    // Grid layout (W = wall, . = open):
    //   . . . . .
    //   . . W . .
    //   . . W . .
    //   . . . . .
    let mut grid = utils::grid(5, 4);
    grid.set_occupied(utils::GROUND, utils::nav(2, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 1),
        utils::world(4, 1),
        0,
    )
    .unwrap();

    // Optimal route goes over the top (cost 40 = 4 × 10, all moves equal cost).
    // (2,0)→(3,1) diagonal is blocked by corner-cutting prevention: (2,1) is a wall.
    assert_eq!(
        path,
        vec![
            utils::world(1, 0),
            utils::world(2, 0),
            utils::world(3, 0),
            utils::world(4, 1)
        ]
    );
}

#[test]
fn returns_none_for_blocked_goal() {
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(4, 4), true);

    assert!(
        astar::find_path(
            &grid,
            Projection::Isometric,
            utils::GROUND,
            utils::world(0, 0),
            utils::world(4, 4),
            0
        )
        .is_none()
    );
}

#[test]
fn empty_path_when_already_at_goal() {
    let grid = utils::grid(5, 5);
    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(2, 2),
        utils::world(2, 2),
        0,
    )
    .unwrap();

    assert!(path.is_empty());
}

//
// ─── Isometric — stop distance ────────────────────────────────────────────────
//

#[test]
fn isometric_empty_path_when_already_within_stop_distance() {
    let grid = utils::grid(10, 10);
    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(3, 0),
        utils::world(5, 0),
        3,
    )
    .unwrap();

    assert!(path.is_empty());
}

#[test]
fn isometric_path_ends_within_stop_distance() {
    let grid = utils::grid(10, 10);
    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(9, 0),
        2,
    )
    .unwrap();

    let last = NavPos::from(*path.last().unwrap());
    let dx = last.x.abs_diff(9);
    let dy = last.y.abs_diff(0);
    assert!(dx.max(dy) <= 2);
}

/// A blocked goal is reachable via stop distance (unit stops adjacent).
#[test]
fn isometric_stop_distance_allows_blocked_goal() {
    // S . . . .   S = start, X = blocked goal
    // . . . . .   * = valid stop positions (distance 1)
    // . . . . .
    // . . . * *
    // . . . * X
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(4, 4), true);

    let path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(4, 4),
        1,
    )
    .unwrap();
    // Nearest stop position is (3,3) at Chebyshev distance 1 from the blocked goal.
    // Optimal: three diagonals (cost 30 = 3 × 10).
    assert_eq!(
        path,
        vec![utils::world(1, 1), utils::world(2, 2), utils::world(3, 3)]
    );
}

//
// ─── Orthogonal — exact goal ──────────────────────────────────────────────────
//

#[test]
fn orthogonal_routes_around_wall() {
    // Same wall as isometric_routes_around_wall; diagonal now costs 14.
    //   . . . . .
    //   . . W . .
    //   . . W . .
    //   . . . . .
    let mut grid = utils::grid(5, 4);
    grid.set_occupied(utils::GROUND, utils::nav(2, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let path = astar::find_path(
        &grid,
        Projection::Orthogonal,
        utils::GROUND,
        utils::world(0, 1),
        utils::world(4, 1),
        0,
    )
    .unwrap();

    // Optimal route still goes over the top (cost 48 = diag+card+card+diag = 14+10+10+14).
    assert_eq!(
        path,
        vec![
            utils::world(1, 0),
            utils::world(2, 0),
            utils::world(3, 0),
            utils::world(4, 1)
        ]
    );
}

//
// ─── Orthogonal — stop distance ───────────────────────────────────────────────
//

#[test]
fn orthogonal_stop_within_euclidean_range() {
    // start=(0,0), goal=(4,3), stop_distance=5
    // Euclidean: √(16+9) = √25 = 5 ≤ 5 → already in range → empty path.
    // (3-4-5 right triangle)
    let grid = utils::grid(10, 10);

    let path = astar::find_path(
        &grid,
        Projection::Orthogonal,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(4, 3),
        5,
    )
    .unwrap();

    assert!(path.is_empty());
}

//
// ─── Projection comparison ────────────────────────────────────────────────────
//

#[test]
fn orthogonal_in_range_uses_euclidean() {
    // start=(0,3), goal=(4,0), stop_distance=4
    //
    // Chebyshev: max(4, 3) = 4 ≤ 4 → Isometric already in range (empty path)
    // Euclidean: √(16+9) = 5 > 4   → Orthogonal not in range (path needed)
    let grid = utils::grid(10, 10);

    let iso_path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 3),
        utils::world(4, 0),
        4,
    )
    .unwrap();

    let ortho_path = astar::find_path(
        &grid,
        Projection::Orthogonal,
        utils::GROUND,
        utils::world(0, 3),
        utils::world(4, 0),
        4,
    )
    .unwrap();

    assert!(iso_path.is_empty());
    // (1,2) is the first step: Euclidean²=(1-4)²+(2-0)²=13≤16, so Orthogonal stops there.
    assert_eq!(ortho_path, vec![utils::world(1, 2)]);
}

#[test]
fn projections_stop_at_different_positions() {
    // start=(0,4), goal=(5,0), stop_distance=4
    //
    // Isometric  (Chebyshev ≤ 4): (1,4) is in range — max(|1-5|,|4-0|)=4≤4 — reached in one step.
    // Orthogonal (Euclidean ≤ 4): (1,4) is NOT in range — √(16+16)≈5.7>4 — must move closer to (2,2).
    let grid = utils::grid(10, 10);

    let iso_path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 4),
        utils::world(5, 0),
        4,
    )
    .unwrap();

    let ortho_path = astar::find_path(
        &grid,
        Projection::Orthogonal,
        utils::GROUND,
        utils::world(0, 4),
        utils::world(5, 0),
        4,
    )
    .unwrap();

    assert_eq!(iso_path, vec![utils::world(1, 4)]);
    assert_eq!(ortho_path, vec![utils::world(1, 3), utils::world(2, 2)]);
}

//
// ─── Layer mask ───────────────────────────────────────────────────────────────
//

#[test]
fn layer_mask_filters_obstacles() {
    // . . W . .   W = GROUND obstacle at (2,0); AIR units ignore it.
    let mut grid = utils::grid(5, 3);
    grid.set_occupied(utils::GROUND, utils::nav(2, 0), true);

    let air_path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::AIR,
        utils::world(0, 0),
        utils::world(4, 0),
        0,
    )
    .unwrap();

    let ground_path = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(4, 0),
        0,
    )
    .unwrap();

    // AIR: straight line through (2,0).
    assert_eq!(
        air_path,
        vec![
            utils::world(1, 0),
            utils::world(2, 0),
            utils::world(3, 0),
            utils::world(4, 0)
        ]
    );
    // GROUND: detours over the top, stepping through y=1.
    assert_eq!(
        ground_path,
        vec![
            utils::world(1, 1),
            utils::world(2, 1),
            utils::world(3, 1),
            utils::world(4, 0)
        ]
    );
}

//
// ─── Determinism ─────────────────────────────────────────────────────────────
//

#[test]
fn is_deterministic() {
    let mut grid = utils::grid(8, 8);
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);
    grid.set_occupied(utils::GROUND, utils::nav(3, 4), true);

    let p1 = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(7, 7),
        0,
    )
    .unwrap();
    let p2 = astar::find_path(
        &grid,
        Projection::Isometric,
        utils::GROUND,
        utils::world(0, 0),
        utils::world(7, 7),
        0,
    )
    .unwrap();

    assert_eq!(p1, p2);
}
