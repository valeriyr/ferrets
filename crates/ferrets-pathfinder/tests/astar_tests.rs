//! A* pathfinding: shortest-path search, range checks, and distance metrics.

mod utils;

use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::{
    astar::{self, Projection},
    nav_grid::NavGrid,
    nav_pos::NavPos,
    nav_size::NavSize,
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
    // Isometric (Chebyshev   max(2,2)=2 ≤ 2): reaches (2,2) → Some([(2,2)])
    // Orthogonal (Euclidean  √8  ≈2.83 > 2):  falls short → None
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
// ─── in_range ─────────────────────────────────────────────────────────────────
//
// Distance between two single cells:
//
// . . . . . . .   y=0
// . . . . . . .   y=1
// . . A . . . .   y=2   A = (2,2), B = (5,5)
// . . . . . . .   y=3   Isometric(Chebyshev):  max(3,3) = 3
// . . . . . . .   y=4   Orthogonal(Euclidean): √(3²+3²) ≈ 4.24
// . . . . . B .   y=5
// . . . . . . .   y=6
//

#[test]
fn in_range_zero_distance_requires_exact_position() {
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(astar::in_range(
            projection,
            utils::nav(3, 3),
            utils::nav(3, 3),
            0
        ));
        assert!(!astar::in_range(
            projection,
            utils::nav(3, 3),
            utils::nav(4, 3),
            0
        ));
    }
}

#[test]
fn in_range_cardinal_distance_matches_in_both_projections() {
    // (3,0) → (5,0): 2 cells apart on one axis — both metrics agree.
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(astar::in_range(
            projection,
            utils::nav(3, 0),
            utils::nav(5, 0),
            2
        ));
        assert!(!astar::in_range(
            projection,
            utils::nav(3, 0),
            utils::nav(5, 0),
            1
        ));
    }
}

#[test]
fn in_range_diagonal_is_chebyshev_for_isometric() {
    // (2,2) → (5,5): Chebyshev max(3,3) = 3.
    assert!(astar::in_range(
        Projection::Isometric,
        utils::nav(2, 2),
        utils::nav(5, 5),
        3
    ));
    assert!(!astar::in_range(
        Projection::Isometric,
        utils::nav(2, 2),
        utils::nav(5, 5),
        2
    ));
}

#[test]
fn in_range_diagonal_is_euclidean_for_orthogonal() {
    // (2,2) → (5,5): Euclidean √18 ≈ 4.24 — more than 4, within 5.
    assert!(!astar::in_range(
        Projection::Orthogonal,
        utils::nav(2, 2),
        utils::nav(5, 5),
        4
    ));
    assert!(astar::in_range(
        Projection::Orthogonal,
        utils::nav(2, 2),
        utils::nav(5, 5),
        5
    ));
}

#[test]
fn in_range_pythagorean_triple_is_exact_for_orthogonal() {
    // (0,3) → (4,0): a 3-4-5 triangle — Euclidean distance exactly 5; Chebyshev 4.
    assert!(astar::in_range(
        Projection::Orthogonal,
        utils::nav(0, 3),
        utils::nav(4, 0),
        5
    ));
    assert!(!astar::in_range(
        Projection::Orthogonal,
        utils::nav(0, 3),
        utils::nav(4, 0),
        4
    ));
    assert!(astar::in_range(
        Projection::Isometric,
        utils::nav(0, 3),
        utils::nav(4, 0),
        4
    ));
}

//
// ─── in_range_of_rect ─────────────────────────────────────────────────────────
//
// All tests use a 2×2 rectangle at origin (3,3):
//
// . . . . . . .   y=0
// . . . . . . .   y=1
// . . . . . . .   y=2
// . . . R R . .   y=3   R = rectangle cells (3,3) (4,3)
// . . . R R . .   y=4                       (3,4) (4,4)
// . . . . . . .   y=5
// . . . . . . .   y=6
//

#[test]
fn rect_position_inside_is_at_distance_zero() {
    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        assert!(in_range_of_rect_iso(cell, 0));
        assert!(in_range_of_rect_ortho(cell, 0));
    }
}

#[test]
fn rect_cardinally_adjacent_position_is_at_distance_one() {
    // Touching each side: left of (3,3), right of (4,4), above (4,3), below (3,4).
    for cell in [(2, 3), (5, 4), (4, 2), (3, 5)] {
        assert!(!in_range_of_rect_iso(cell, 0));
        assert!(in_range_of_rect_iso(cell, 1));
        assert!(!in_range_of_rect_ortho(cell, 0));
        assert!(in_range_of_rect_ortho(cell, 1));
    }
}

#[test]
fn rect_distance_is_measured_to_nearest_cell_not_origin() {
    // (6,4) is 3 from the origin (3,3) but only 2 from the nearest cell (4,4).
    assert!(in_range_of_rect_iso((6, 4), 2));
    assert!(in_range_of_rect_ortho((6, 4), 2));
    assert!(!in_range_of_rect_iso((6, 4), 1));
    assert!(!in_range_of_rect_ortho((6, 4), 1));
}

#[test]
fn rect_isometric_treats_diagonal_corner_as_distance_one() {
    // (2,2) touches the rect corner (3,3) diagonally — Chebyshev 1.
    assert!(in_range_of_rect_iso((2, 2), 1));
}

#[test]
fn rect_orthogonal_treats_diagonal_corner_as_farther_than_one() {
    // (2,2) → nearest cell (3,3): Euclidean √2 > 1, but ≤ 2.
    assert!(!in_range_of_rect_ortho((2, 2), 1));
    assert!(in_range_of_rect_ortho((2, 2), 2));
}

#[test]
fn rect_corner_clamps_on_both_axes() {
    // (6,6) → nearest cell (4,4): Chebyshev 2; Euclidean √8 ≈ 2.83.
    assert!(in_range_of_rect_iso((6, 6), 2));
    assert!(!in_range_of_rect_ortho((6, 6), 2));
    assert!(in_range_of_rect_ortho((6, 6), 3));
}

#[test]
fn rect_single_cell_matches_in_range() {
    // A 1×1 rectangle degenerates to a plain point-range check.
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        for from in [(3, 3), (1, 3), (0, 0), (6, 2)] {
            for distance in 0..4 {
                assert_eq!(
                    astar::in_range_of_rect(
                        projection,
                        utils::nav(from.0, from.1),
                        RECT_ORIGIN,
                        NavSize::ONE,
                        distance,
                    ),
                    astar::in_range(
                        projection,
                        utils::nav(from.0, from.1),
                        RECT_ORIGIN,
                        distance
                    ),
                    "projection {projection:?}, from {from:?}, distance {distance}"
                );
            }
        }
    }
}

#[test]
fn rect_wide_clamps_to_facing_side() {
    // 3×1 rectangle: cells (3,3) (4,3) (5,3).
    // (4,5) is below the middle cell (4,3): distance 2 on both metrics.
    let size = NavSize::new(3, 1);
    let from = utils::nav(4, 5);

    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(astar::in_range_of_rect(
            projection,
            from,
            RECT_ORIGIN,
            size,
            2
        ));
        assert!(!astar::in_range_of_rect(
            projection,
            from,
            RECT_ORIGIN,
            size,
            1
        ));
    }
}

//
// ─── rect_distance ────────────────────────────────────────────────────────────
//
// The first two tests use the same 2×2 rectangle at origin (3,3), with P the
// probe point (7,4) whose nearest rectangle cell is (4,4):
//
// . . . . . . . . .   y=0
// . . . . . . . . .   y=1
// . . . . . . . . .   y=2
// . . . R R . . . .   y=3   R = rectangle cells (3,3) (4,3)
// . . . R R . . P .   y=4                       (3,4) (4,4)
// . . . . . . . . .   y=5
// . . . . . . . . .   y=6
//

#[test]
fn rect_distance_is_zero_inside_footprint() {
    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        assert_eq!(rect_distance_iso(cell), 0);
        assert_eq!(rect_distance_ortho(cell), 0);
    }
}

#[test]
fn rect_distance_measures_to_nearest_cell() {
    // (7,4) → nearest cell (4,4): 3 cells away along the x axis.
    assert_eq!(rect_distance_iso((7, 4)), 3); // Chebyshev cells
    assert_eq!(rect_distance_ortho((7, 4)), 9); // squared Euclidean cells
}

// Against a single-cell rect R at the origin, two candidate cells A and B:
//
// R . . . . . .   y=0   R = rectangle cell (0,0)
// . . . . . . .   y=1   A = diagonal candidate (5,5)
// . . . . . . .   y=2   B = cardinal candidate (0,6)
// . . . . . . .   y=3
// . . . . . . .   y=4
// . . . . . A .   y=5   A: Chebyshev 5, Euclidean² 50
// B . . . . . .   y=6   B: Chebyshev 6, Euclidean² 36
//
// Isometric ranks the diagonal candidate A closer; Orthogonal, which charges
// diagonals more ground, ranks the cardinal candidate B closer.
#[test]
fn rect_distance_ranks_diagonal_vs_cardinal_by_projection() {
    let a = utils::nav(5, 5);
    let b = utils::nav(0, 6);
    let origin = NavPos::new(0, 0);

    assert!(
        astar::rect_distance(Projection::Isometric, a, origin, NavSize::ONE)
            < astar::rect_distance(Projection::Isometric, b, origin, NavSize::ONE)
    );
    assert!(
        astar::rect_distance(Projection::Orthogonal, a, origin, NavSize::ONE)
            > astar::rect_distance(Projection::Orthogonal, b, origin, NavSize::ONE)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Origin of the rectangle used by the `in_range_of_rect` tests.
const RECT_ORIGIN: NavPos = NavPos::new(3, 3);
/// Footprint of the rectangle used by the `in_range_of_rect` tests.
const RECT_SIZE: NavSize = NavSize::new(2, 2);

fn in_range_of_rect_iso(from: (u32, u32), distance: u32) -> bool {
    astar::in_range_of_rect(
        Projection::Isometric,
        utils::nav(from.0, from.1),
        RECT_ORIGIN,
        RECT_SIZE,
        distance,
    )
}

fn in_range_of_rect_ortho(from: (u32, u32), distance: u32) -> bool {
    astar::in_range_of_rect(
        Projection::Orthogonal,
        utils::nav(from.0, from.1),
        RECT_ORIGIN,
        RECT_SIZE,
        distance,
    )
}

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

fn rect_distance_iso(from: (u32, u32)) -> u32 {
    astar::rect_distance(
        Projection::Isometric,
        utils::nav(from.0, from.1),
        RECT_ORIGIN,
        RECT_SIZE,
    )
}

fn rect_distance_ortho(from: (u32, u32)) -> u32 {
    astar::rect_distance(
        Projection::Orthogonal,
        utils::nav(from.0, from.1),
        RECT_ORIGIN,
        RECT_SIZE,
    )
}
