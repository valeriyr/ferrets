//! Fog of war grid: reveal, aging, and per-player independence. Team-combined
//! visibility (allied sharing) is covered by the integration fog tests, which
//! have a live session to resolve alliances against.

use ferrets_simulation::visibility::{CellVisibility, VisibilityGrid};

#[test]
fn new_grid_is_all_unexplored() {
    let grid = VisibilityGrid::new(2, 8, 8);
    for player in 0..2 {
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(grid.get(player, x, y), CellVisibility::Unexplored);
            }
        }
    }
}

#[test]
fn reveal_marks_cell_visible() {
    let mut grid = VisibilityGrid::new(1, 8, 8);
    grid.reveal(0, 3, 4);
    assert_eq!(grid.get(0, 3, 4), CellVisibility::Visible);
}

#[test]
fn aging_demotes_visible_to_explored() {
    let mut grid = VisibilityGrid::new(1, 8, 8);
    grid.reveal(0, 3, 4);
    grid.age();
    assert_eq!(grid.get(0, 3, 4), CellVisibility::Explored);
    // Explored stays sticky across further aging; a never-seen cell stays unexplored.
    grid.age();
    assert_eq!(grid.get(0, 3, 4), CellVisibility::Explored);
    assert_eq!(grid.get(0, 0, 0), CellVisibility::Unexplored);
}

#[test]
fn revealing_after_aging_returns_to_visible() {
    let mut grid = VisibilityGrid::new(1, 8, 8);
    grid.reveal(0, 3, 4);
    grid.age();
    grid.reveal(0, 3, 4);
    assert_eq!(grid.get(0, 3, 4), CellVisibility::Visible);
}

#[test]
fn player_grids_are_independent() {
    let mut grid = VisibilityGrid::new(2, 8, 8);
    grid.reveal(0, 3, 4);
    assert_eq!(grid.get(0, 3, 4), CellVisibility::Visible);
    assert_eq!(grid.get(1, 3, 4), CellVisibility::Unexplored);
}

//
// ─── Bounds ───────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "cell (8, 0) out of range (8x8)")]
fn get_beyond_map_bounds_panics() {
    VisibilityGrid::new(1, 8, 8).get(0, 8, 0);
}

#[test]
#[should_panic(expected = "player 2 out of range (0..2)")]
fn get_for_unknown_player_panics() {
    VisibilityGrid::new(2, 8, 8).get(2, 0, 0);
}

#[test]
#[should_panic(expected = "cell (0, 8) out of range (8x8)")]
fn reveal_beyond_map_bounds_panics() {
    VisibilityGrid::new(1, 8, 8).reveal(0, 0, 8);
}

#[test]
#[should_panic(expected = "player 2 out of range (0..2)")]
fn reveal_for_unknown_player_panics() {
    VisibilityGrid::new(2, 8, 8).reveal(2, 0, 0);
}
