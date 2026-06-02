mod utils;

use ferrets_simulation::pathfinding::search;

//
// ─── find_nearest_free_pos ────────────────────────────────────────────────────
//

#[test]
fn returns_self_when_already_passable() {
    let grid = utils::grid(5, 5);

    assert_eq!(
        search::find_nearest_free_pos(&grid, utils::GROUND, utils::nav(2, 2)),
        Some(utils::nav(2, 2))
    );
}

#[test]
fn finds_adjacent_when_start_blocked() {
    // . . . . .
    // . . . . .
    // . . X . .   X = blocked start, result is one of the adjacent dots
    // . . . . .
    // . . . . .
    let mut grid = utils::grid(5, 5);
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    let result = search::find_nearest_free_pos(&grid, utils::GROUND, utils::nav(2, 2)).unwrap();
    let dx = result.x.abs_diff(2);
    let dy = result.y.abs_diff(2);
    assert!(
        dx <= 1 && dy <= 1,
        "result should be adjacent to the blocked start"
    );
}

#[test]
fn finds_free_through_obstacle_ring() {
    // . . . . . . .
    // . . . . . . .
    // . . X X X . .
    // . . X S X . .   S = blocked start, X = blocked ring
    // . . X X X . .
    // . . . . . . .
    // . . . . . . .
    let mut grid = utils::grid(7, 7);
    for dx in -1_i32..=1 {
        for dy in -1_i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            grid.set_occupied(
                utils::GROUND,
                utils::nav((3_i32 + dx) as u32, (3_i32 + dy) as u32),
                true,
            );
        }
    }
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);

    let result = search::find_nearest_free_pos(&grid, utils::GROUND, utils::nav(3, 3));
    assert!(
        result.is_some(),
        "should find a free position outside the ring"
    );
}

#[test]
fn returns_none_when_entire_grid_blocked() {
    // X X
    // X X   every position blocked
    let mut grid = utils::grid(2, 2);

    grid.set_occupied(utils::GROUND, utils::nav(0, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(0, 1), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 1), true);

    assert!(search::find_nearest_free_pos(&grid, utils::GROUND, utils::nav(0, 0)).is_none());
}
