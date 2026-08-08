//! Grid search: finding free positions around cells and footprints.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_pathfinder::{
    nav_grid::NavGrid,
    search::{self, Expansion},
};

//
// ─── find_nearest_free_pos ────────────────────────────────────────────────────
//

#[test]
fn returns_self_when_already_passable() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . S . .   y=2   S = start (2,2) — already passable
    // . . . . .   y=3
    // . . . . .   y=4
    let grid = utils::grid(5, 5);

    let result = find_through_blocked(&grid, utils::nav(2, 2));

    assert_eq!(result, Some(utils::nav(2, 2)));
}

#[test]
fn finds_adjacent_when_start_blocked() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . X . .   y=2   X = blocked start; first direction (0,-1) → (2,1) is passable
    // . . . . .   y=3
    // . . . . .   y=4
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let result = find_through_blocked(&grid, utils::nav(2, 2));

    assert_eq!(result, Some(utils::nav(2, 1)));
}

#[test]
fn through_passable_finds_adjacent_when_start_blocked() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . X . .   y=2   X = blocked start
    // . . . . .   y=3
    // . . . . .   y=4
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let result = find_through_passable(&grid, utils::nav(2, 2));

    assert_eq!(result, Some(utils::nav(2, 1)));
}

#[test]
fn through_blocked_finds_free_outside_ring() {
    // . . . . . . .   y=0
    // . . . . . . .   y=1
    // . . X X X . .   y=2
    // . . X S X . .   y=3   S = blocked start, X = blocked ring
    // . . X X X . .   y=4
    // . . . . . . .   y=5
    // . . . . . . .   y=6
    //
    // ThroughBlocked: BFS expands through the ring cells.
    // Pops (3,2) first (direction order); its first neighbor (3,1) is passable.
    let mut grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 1);
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);

    let result = find_through_blocked(&grid, utils::nav(3, 3));

    assert_eq!(result, Some(utils::nav(3, 1)));
}

#[test]
fn through_passable_returns_none_when_enclosed() {
    // . . . . . . .   y=0
    // . . . . . . .   y=1
    // . . X X X . .   y=2
    // . . X S X . .   y=3   S = blocked start, X = blocked ring
    // . . X X X . .   y=4
    // . . . . . . .   y=5
    // . . . . . . .   y=6
    //
    // ThroughPassable: blocked cells (other than start) are not expanded.
    // The ring cells are enqueued but skipped when popped — no free cell is found.
    let mut grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 1);
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);

    let result = find_through_passable(&grid, utils::nav(3, 3));

    assert!(result.is_none());
}

#[test]
fn through_passable_finds_free_through_ring_gap() {
    // . . . . . . .   y=0
    // . . . . . . .   y=1
    // . . X X X . .   y=2
    // . . X S . . .   y=3   S = blocked start; gap at (4,3) — east ring cell removed
    // . . X X X . .   y=4
    // . . . . . . .   y=5
    // . . . . . . .   y=6
    //
    // ThroughPassable: stops at ring cells, but (4,3) is passable and adjacent → returned.
    let mut grid = utils::hollow_ring_grid(7, utils::nav(3, 3), 1);
    grid.set_occupied(utils::GROUND, utils::nav(4, 3), false);
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);

    let result = find_through_passable(&grid, utils::nav(3, 3));

    assert_eq!(result, Some(utils::nav(4, 3)));
}

#[test]
fn returns_none_when_entire_grid_blocked() {
    // X X   y=0   every position blocked
    // X X   y=1
    let mut grid = utils::grid(2, 2);

    grid.set_occupied(utils::GROUND, utils::nav(0, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(0, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 1), true);

    let result = find_through_blocked(&grid, utils::nav(0, 0));

    assert!(result.is_none());
}

#[test]
fn layer_mask_filters_obstacles() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . X . .   y=2   X = GROUND obstacle; passable on AIR
    // . . . . .   y=3
    // . . . . .   y=4
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    // AIR: (2,2) is passable → returns itself.
    let air_result = search::find_nearest_free_pos(
        &grid,
        utils::AIR,
        utils::nav(2, 2),
        Expansion::ThroughBlocked,
    );

    // GROUND: (2,2) is blocked → first direction (0,-1) → (2,1) is passable.
    let ground_result = find_through_blocked(&grid, utils::nav(2, 2));

    assert_eq!(air_result, Some(utils::nav(2, 2)));
    assert_eq!(ground_result, Some(utils::nav(2, 1)));
}

//
// ─── find_placement_near ──────────────────────────────────────────────────────
//

#[test]
fn placement_returns_free_anchor_cell_first() {
    // The anchor rectangle itself is scanned first; on an empty grid the
    // top-left anchor cell wins.
    let grid = utils::grid(10, 10);

    let found = search::find_placement_near(
        &grid,
        utils::GROUND,
        utils::nav(3, 3),
        CellSize::new(2, 2),
        CellSize::ONE,
        8,
    );

    assert_eq!(found, Some(utils::nav(3, 3)));
}

#[test]
fn placement_skips_occupied_anchor_and_picks_first_ring_cell() {
    // . . . . . .   y=2   anchor 2×2 at (3,3) fully occupied → ring 1 spans
    // . . A A . .   y=3   (2,2)..(5,5); first free cell in row-major order
    // . . A A . .   y=4   is (2,2).
    let mut grid = utils::grid(10, 10);
    for y in 3..5 {
        for x in 3..5 {
            grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
        }
    }

    let found = search::find_placement_near(
        &grid,
        utils::GROUND,
        utils::nav(3, 3),
        CellSize::new(2, 2),
        CellSize::ONE,
        8,
    );

    assert_eq!(found, Some(utils::nav(2, 2)));
}

#[test]
fn placement_requires_whole_spawn_footprint() {
    // Single free cells are not enough for a 2×2 spawn; the first row-major
    // position whose whole 2×2 footprint is free wins.
    let mut grid = utils::grid(8, 8);
    // Occupy the anchor cell and checker the first ring so no 2×2 fits there.
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);
    for &(x, y) in &[(2, 2), (4, 2), (2, 4), (4, 4)] {
        grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
    }

    let found = search::find_placement_near(
        &grid,
        utils::GROUND,
        utils::nav(3, 3),
        CellSize::ONE,
        CellSize::new(2, 2),
        8,
    );

    let origin = found.expect("a 2×2 area exists within the radius");
    for dy in 0..2 {
        for dx in 0..2 {
            assert!(grid.is_passable_by(utils::GROUND, utils::nav(origin.x + dx, origin.y + dy)));
        }
    }
}

#[test]
fn placement_gives_up_beyond_max_radius() {
    // Everything within radius 1 of the anchor is occupied; radius 1 search fails.
    let mut grid = utils::grid(10, 10);
    for y in 2..6 {
        for x in 2..6 {
            grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
        }
    }

    let found = search::find_placement_near(
        &grid,
        utils::GROUND,
        utils::nav(3, 3),
        CellSize::new(2, 2),
        CellSize::ONE,
        1,
    );

    assert_eq!(found, None);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn find_through_blocked(grid: &NavGrid, pos: CellPos) -> Option<CellPos> {
    search::find_nearest_free_pos(grid, utils::GROUND, pos, Expansion::ThroughBlocked)
}

fn find_through_passable(grid: &NavGrid, pos: CellPos) -> Option<CellPos> {
    search::find_nearest_free_pos(grid, utils::GROUND, pos, Expansion::ThroughPassable)
}
