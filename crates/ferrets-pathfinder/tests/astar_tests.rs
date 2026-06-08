mod utils;

use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::{
    astar::{self, Projection},
    nav_grid::NavGrid,
};

//
// ─── Isometric — exact goal ───────────────────────────────────────────────────
//

#[test]
fn straight_line_on_open_grid() {
    // S . . G . . . . . .   y=0   S = start (0,0), G = goal (3,0)
    let grid = utils::grid(10, 10);

    let path = find_iso(&grid, utils::world(0, 0), utils::world(3, 0), 0).unwrap();

    assert_eq!(
        path,
        vec![utils::world(1, 0), utils::world(2, 0), utils::world(3, 0)]
    );
}

#[test]
fn diagonal_blocked_when_both_adjacent_cardinals_are_walls() {
    // . . .   y=0
    // X S .   y=1   X = blocked (0,1), S = start (1,1)
    // G X .   y=2   G = goal (0,2), X = blocked (1,2)
    //
    // Move (1,1)→(0,2): both adjacent cardinals (0,1) and (1,2) are walls.
    // (0,2) has no other reachable neighbor — result is None.
    let mut grid = utils::grid(3, 3);
    grid.set_occupied(utils::GROUND, utils::nav(0, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 2), true);

    let path = find_iso(&grid, utils::world(1, 1), utils::world(0, 2), 0);

    assert!(path.is_none());
}

#[test]
fn returns_none_for_blocked_goal() {
    // S . . . .   y=0
    // . . . . .   y=1
    // . . . . .   y=2
    // . . . . .   y=3
    // . . . . X   y=4   S = start (0,0), X = blocked goal (4,4)
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(4, 4), true);

    let path = find_iso(&grid, utils::world(0, 0), utils::world(4, 4), 0);

    assert!(path.is_none());
}

#[test]
fn empty_path_when_already_at_goal() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . S . .   y=2   S = start = goal (2,2)
    // . . . . .   y=3
    // . . . . .   y=4
    let grid = utils::grid(5, 5);

    let path = find_iso(&grid, utils::world(2, 2), utils::world(2, 2), 0).unwrap();

    assert!(path.is_empty());
}

#[test]
fn empty_path_when_already_at_occupied_goal() {
    // Unit is standing on a cell that became occupied after it moved there.
    // start == goal and the cell is blocked — should return Some([]), not None.
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let path = find_iso(&grid, utils::world(2, 2), utils::world(2, 2), 0).unwrap();

    assert!(path.is_empty());
}

//
// ─── Stop distance ────────────────────────────────────────────────────────────
//

#[test]
fn isometric_empty_path_when_already_within_stop_distance() {
    // . . . S . G . . . .   y=0   stop_distance=3, Chebyshev = max(2,0) = 2 ≤ 3 → already in range
    let grid = utils::grid(10, 10);

    let path = find_iso(&grid, utils::world(3, 0), utils::world(5, 0), 3).unwrap();

    assert!(path.is_empty());
}

#[test]
fn isometric_path_ends_within_stop_distance() {
    // S . . . . . . . . G   y=0   S = start (0,0), G = goal (9,0), stop_distance=2
    //
    // A* follows y=0 straight: each step keeps f=70 and g strictly increases,
    // winning every tie against diagonal branches. Stops at (7,0) where
    // isometric distance to goal = 2.
    let grid = utils::grid(10, 10);

    let path = find_iso(&grid, utils::world(0, 0), utils::world(9, 0), 2).unwrap();

    assert_eq!(
        path,
        vec![
            utils::world(1, 0),
            utils::world(2, 0),
            utils::world(3, 0),
            utils::world(4, 0),
            utils::world(5, 0),
            utils::world(6, 0),
            utils::world(7, 0),
        ]
    );
}

/// A blocked goal is reachable via stop distance (unit stops adjacent).
#[test]
fn isometric_stop_distance_allows_blocked_goal() {
    // S . . . .   y=0   S = start (0,0)
    // . . . . .   y=1
    // . . . . .   y=2
    // . . . * *   y=3   * = valid stop positions (distance 1)
    // . . . * X   y=4   X = blocked goal (4,4)
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(4, 4), true);

    let path = find_iso(&grid, utils::world(0, 0), utils::world(4, 4), 1).unwrap();
    // Nearest stop position is (3,3) at isometric distance 1 from the blocked goal.
    // Optimal: three diagonals (cost 30 = 3 × 10).
    assert_eq!(
        path,
        vec![utils::world(1, 1), utils::world(2, 2), utils::world(3, 3)]
    );
}

#[test]
fn orthogonal_stop_within_euclidean_range() {
    // S . . . . . . . . .   y=0
    // . . . . . . . . . .   y=1
    // . . . . . . . . . .   y=2
    // . . . . G . . . . .   y=3   S = start (0,0), G = goal (4,3); √(4²+3²) = 5 ≤ stop_distance=5
    let grid = utils::grid(10, 10);

    let path = find_ortho(&grid, utils::world(0, 0), utils::world(4, 3), 5).unwrap();

    assert!(path.is_empty());
}

//
// ─── Projection comparison ────────────────────────────────────────────────────
//

#[test]
fn routes_around_wall() {
    // . . . . .   y=0
    // S . X . G   y=1   S = start (0,1), G = goal (4,1)
    // . . X . .   y=2
    // . . . . .   y=3
    //
    // Both projections route over the top: (0,1)→(1,0)→(2,0)→(3,0)→(4,1).
    // (2,0)→(3,1) diagonal is blocked by corner-cutting prevention: (2,1) is blocked.
    // Isometric cost = 40 (4 × 10, all moves equal).
    // Orthogonal cost = 48 (14+10+10+14, diagonals cost more).
    let mut grid = utils::grid(5, 4);
    grid.set_occupied(utils::GROUND, utils::nav(2, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let expected = vec![
        utils::world(1, 0),
        utils::world(2, 0),
        utils::world(3, 0),
        utils::world(4, 1),
    ];

    let iso_path = find_iso(&grid, utils::world(0, 1), utils::world(4, 1), 0).unwrap();
    let ortho_path = find_ortho(&grid, utils::world(0, 1), utils::world(4, 1), 0).unwrap();

    assert_eq!(iso_path, expected);
    assert_eq!(ortho_path, expected);
}

#[test]
fn orthogonal_in_range_uses_euclidean() {
    // . . . . G . . . . .   y=0
    // . . . . . . . . . .   y=1
    // . . . . . . . . . .   y=2
    // S . . . . . . . . .   y=3   S = start (0,3), G = goal (4,0), stop_distance=4
    //
    // Chebyshev: max(4, 3) = 4 ≤ 4 → Isometric already in range (empty path)
    // Euclidean: √(16+9) = 5 > 4   → Orthogonal not in range (path needed)
    let grid = utils::grid(10, 10);

    let iso_path = find_iso(&grid, utils::world(0, 3), utils::world(4, 0), 4).unwrap();
    let ortho_path = find_ortho(&grid, utils::world(0, 3), utils::world(4, 0), 4).unwrap();

    assert!(iso_path.is_empty());
    // (1,2) is the first step: Euclidean²=(1-4)²+(2-0)²=13≤16, so Orthogonal stops there.
    assert_eq!(ortho_path, vec![utils::world(1, 2)]);
}

#[test]
fn projections_stop_at_different_positions() {
    // . . . . . G . . . .   y=0
    // . . . . . . . . . .   y=1
    // . . . . . . . . . .   y=2
    // . . . . . . . . . .   y=3
    // S . . . . . . . . .   y=4   S = start (0,4), G = goal (5,0), stop_distance=4
    //
    // Isometric  (Chebyshev ≤ 4): (1,4) is in range — max(|1-5|,|4-0|)=4≤4 — reached in one step.
    // Orthogonal (Euclidean ≤ 4): (1,4) is NOT in range — √(16+16)≈5.7>4 — must move closer to (2,2).
    let grid = utils::grid(10, 10);

    let iso_path = find_iso(&grid, utils::world(0, 4), utils::world(5, 0), 4).unwrap();
    let ortho_path = find_ortho(&grid, utils::world(0, 4), utils::world(5, 0), 4).unwrap();

    assert_eq!(iso_path, vec![utils::world(1, 4)]);
    assert_eq!(ortho_path, vec![utils::world(1, 3), utils::world(2, 2)]);
}

//
// ─── Layer mask ───────────────────────────────────────────────────────────────
//

#[test]
fn layer_mask_filters_obstacles() {
    // S . X . G   y=0   S = start (0,0), G = goal (4,0), X = GROUND obstacle; AIR ignores it
    // . . . . .   y=1
    // . . . . .   y=2
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

    let ground_path = find_iso(&grid, utils::world(0, 0), utils::world(4, 0), 0).unwrap();

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
// ─── Enclosed start (ring) ───────────────────────────────────────────────────
//
//  Grid (7 × 7):
//
// G . . . . . .   y=0   G = goal (0,0)
// . X X X X X .   y=1
// . X . . . X .   y=2
// . X . S . X .   y=3   S = start (3,3); open interior: (2,2)..(4,4)
// . X . . . X .   y=4
// . X X X X X .   y=5
// . . . . . . .   y=6
//
//  Nearest reachable interior cell (2,2):
//    isometric distance: max(2,2) = 2
//    orthogonal distance: √(2²+2²) = √8 ≈ 2.83

#[test]
fn returns_none_when_enclosed_exact_goal() {
    // stop_distance=0: goal (0,0) is outside the ring and unreachable — start is enclosed.
    let grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 2);

    let path = find_iso(&grid, utils::world(3, 3), utils::world(0, 0), 0);

    assert!(path.is_none());
}

#[test]
fn returns_none_when_enclosed_stop_distance_1() {
    // Nearest reachable interior cell (2,2) is at isometric distance 2 — outside stop range of 1.
    let grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 2);

    let path = find_iso(&grid, utils::world(3, 3), utils::world(0, 0), 1);

    assert!(path.is_none());
}

#[test]
fn projections_differ_at_stop_distance_2() {
    // stop_distance=2, goal=(0,0): (2,2) is the nearest reachable interior cell.
    //
    // Isometric (Chebyshev max(2,2)=2 ≤ 2): reaches (2,2) → Some([(2,2)])
    // Orthogonal (Euclidean    √8  ≈2.83 > 2): falls short → None
    let grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 2);

    let iso_path = find_iso(&grid, utils::world(3, 3), utils::world(0, 0), 2);
    let ortho_path = find_ortho(&grid, utils::world(3, 3), utils::world(0, 0), 2);

    assert_eq!(iso_path.unwrap(), vec![utils::world(2, 2)]);
    assert!(ortho_path.is_none());
}

#[test]
fn orthogonal_stops_inside_ring_when_within_stop_distance() {
    // stop_distance=3: (2,2) is the unique interior cell within orthogonal distance 3 of goal (0,0).
    let grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 2);

    let path = find_ortho(&grid, utils::world(3, 3), utils::world(0, 0), 3).unwrap();

    assert_eq!(path, vec![utils::world(2, 2)]);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn find_iso(
    grid: &NavGrid,
    start: FixedUVec2,
    goal: FixedUVec2,
    distance: u32,
) -> Option<Vec<FixedUVec2>> {
    astar::find_path(
        grid,
        Projection::Isometric,
        utils::GROUND,
        start,
        goal,
        distance,
    )
}

fn find_ortho(
    grid: &NavGrid,
    start: FixedUVec2,
    goal: FixedUVec2,
    distance: u32,
) -> Option<Vec<FixedUVec2>> {
    astar::find_path(
        grid,
        Projection::Orthogonal,
        utils::GROUND,
        start,
        goal,
        distance,
    )
}
