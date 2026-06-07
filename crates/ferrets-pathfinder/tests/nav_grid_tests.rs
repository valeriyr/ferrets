mod utils;

use ferrets_pathfinder::nav_grid::NavGrid;

//
// ─── Layers ───────────────────────────────────────────────────────────────────
//

#[test]
fn layers_are_independent() {
    // GROUND layer:    AIR layer:
    // . . . . .        . . . . .
    // . . . . .        . . . . .
    // . . X . .        . . . . .
    // . . . . .        . . . . .
    // . . . . .        . . . . .
    let mut grid = utils::grid(5, 5);

    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    assert!(grid.is_occupied(utils::GROUND, utils::nav(2, 2)));
    assert!(grid.is_passable(utils::AIR, utils::nav(2, 2)));
}

#[test]
#[should_panic]
fn add_layer_panics_on_duplicate() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.add_layer(utils::GROUND);
}

// The following five tests rely on `debug_assert!` inside `assert_registered` and only
// fire in debug builds. They are excluded from release-mode test runs.

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn set_occupied_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.set_occupied(utils::AIR, utils::nav(1, 1), true);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn is_occupied_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.is_occupied(utils::AIR, utils::nav(1, 1));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn is_occupied_by_panics_on_unregistered_mask() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.is_occupied_by(utils::GROUND | utils::AIR, utils::nav(1, 1));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn is_passable_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.is_passable(utils::AIR, utils::nav(1, 1));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn is_passable_by_panics_on_unregistered_mask() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);
    grid.is_passable_by(utils::GROUND | utils::AIR, utils::nav(1, 1));
}

//
// ─── set_occupied / is_passable ─────────────────────────────────────────────
//

#[test]
fn all_positions_passable_after_add_layer() {
    let mut grid = NavGrid::new(4, 4);
    grid.add_layer(utils::GROUND);

    assert!(grid.is_passable(utils::GROUND, utils::nav(0, 0)));
    assert!(grid.is_passable(utils::GROUND, utils::nav(3, 3)));
}

#[test]
fn set_toggles_single_position() {
    let mut grid = NavGrid::new(5, 5);
    grid.add_layer(utils::GROUND);

    let pos = utils::nav(2, 2);

    grid.set_occupied(utils::GROUND, pos, true);
    assert!(grid.is_occupied(utils::GROUND, pos));

    grid.set_occupied(utils::GROUND, pos, false);
    assert!(grid.is_passable(utils::GROUND, pos));
}

#[test]
fn out_of_bounds_is_impassable() {
    let mut grid = NavGrid::new(3, 3);
    grid.add_layer(utils::GROUND);

    assert!(!grid.is_passable(utils::GROUND, utils::nav(3, 0)));
    assert!(!grid.is_passable(utils::GROUND, utils::nav(0, 3)));
}

//
// ─── is_passable_by ───────────────────────────────────────────────────────────
//

#[test]
fn is_passable_by_requires_all_layers() {
    // GROUND layer:    AIR layer:     GROUND|AIR at (1,1):
    // . . . . .        . . . . .      blocked (GROUND is occupied)
    // . X . . .        . . . . .
    // . . . . .        . . . . .
    let mut grid = utils::grid(5, 5);

    grid.set_occupied(utils::GROUND, utils::nav(1, 1), true);

    assert!(!grid.is_passable_by(utils::GROUND | utils::AIR, utils::nav(1, 1)));
    assert!(grid.is_passable_by(utils::AIR, utils::nav(1, 1)));
}

//
// ─── is_occupied / is_occupied_by ──────────────────────────────────────────
//

#[test]
fn is_occupied_returns_true_on_set_layer() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied(utils::GROUND, utils::nav(2, 2), true);

    assert!(grid.is_occupied(utils::GROUND, utils::nav(2, 2)));
    assert!(!grid.is_occupied(utils::AIR, utils::nav(2, 2)));
}

#[test]
fn is_occupied_by_returns_true_when_any_layer_is_occupied() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied(utils::GROUND, utils::nav(1, 1), true);

    assert!(grid.is_occupied_by(utils::GROUND | utils::AIR, utils::nav(1, 1)));
    assert!(!grid.is_occupied_by(utils::AIR, utils::nav(1, 1)));
}

//
// ─── set_occupied_by ──────────────────────────────────────────────────────────
//

#[test]
fn set_occupied_by_updates_all_matched_layers() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied_by(utils::GROUND | utils::AIR, utils::nav(3, 3), true);

    assert!(grid.is_occupied(utils::GROUND, utils::nav(3, 3)));
    assert!(grid.is_occupied(utils::AIR, utils::nav(3, 3)));
}

#[test]
fn set_occupied_by_does_not_affect_unmasked_layers() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied_by(utils::GROUND, utils::nav(3, 3), true);

    assert!(grid.is_occupied(utils::GROUND, utils::nav(3, 3)));
    assert!(grid.is_passable(utils::AIR, utils::nav(3, 3)));
}
