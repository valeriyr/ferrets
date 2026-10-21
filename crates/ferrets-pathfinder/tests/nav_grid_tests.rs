//! `NavGrid` terrain, layer occupancy, unit claims and shape fits.

mod utils;

use ferrets_geometry::cell_size::CellSize;
use ferrets_pathfinder::{layer_mask::LayerMask, mover_shape::MoverShape, nav_grid::NavGrid};

//
// ─── Layers ───────────────────────────────────────────────────────────────────
//

#[test]
fn layers_are_independent() {
    // . . . . .   y=0
    // . . . . .   y=1
    // . . X . .   y=2   X = (2,2) occupied on GROUND; AIR stays free
    // . . . . .   y=3
    // . . . . .   y=4
    let mut grid = utils::grid(5, 5);

    grid.set_occupied_by(utils::GROUND, utils::nav(2, 2), true);

    assert!(grid.is_occupied_by(utils::GROUND, utils::nav(2, 2)));
    assert!(!grid.is_occupied_by(utils::AIR, utils::nav(2, 2)));
}

#[test]
#[should_panic(expected = "is already registered")]
fn add_layer_panics_on_duplicate() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_layer(utils::GROUND);
    grid.add_layer(utils::GROUND);
}

// The following tests rely on `debug_assert!` inside `assert_registered` and only
// fire in debug builds. They are excluded from release-mode test runs.

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn set_occupied_by_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_layer(utils::GROUND);
    grid.set_occupied_by(utils::AIR, utils::nav(1, 1), true);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn is_occupied_by_panics_on_unregistered_mask() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_layer(utils::GROUND);
    grid.is_occupied_by(utils::GROUND | utils::AIR, utils::nav(1, 1));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn set_claimed_by_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_layer(utils::GROUND);
    grid.set_claimed_by(utils::AIR, utils::nav(1, 1), true);
}

//
// ─── Occupancy ────────────────────────────────────────────────────────────────
//

#[test]
fn every_position_is_free_after_add_layer() {
    let grid = utils::grid(4, 4);

    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(0, 0)));
    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(3, 3)));
}

#[test]
fn set_occupied_by_toggles_single_position() {
    let mut grid = utils::grid(5, 5);
    let pos = utils::nav(2, 2);

    grid.set_occupied_by(utils::GROUND, pos, true);
    assert!(grid.is_occupied_by(utils::GROUND, pos));

    grid.set_occupied_by(utils::GROUND, pos, false);
    assert!(!grid.is_occupied_by(utils::GROUND, pos));
}

#[test]
fn set_occupied_by_ignores_out_of_bounds() {
    let mut grid = utils::grid(3, 3);

    grid.set_occupied_by(utils::GROUND, utils::nav(3, 0), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(0, 3), true);

    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(2, 0)));
    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(0, 2)));
}

#[test]
fn out_of_bounds_is_occupied() {
    let grid = utils::grid(3, 3);

    assert!(grid.is_occupied_by(utils::GROUND, utils::nav(3, 0)));
    assert!(grid.is_statically_occupied_by(utils::GROUND, utils::nav(0, 3)));
}

#[test]
fn is_occupied_by_returns_true_when_any_layer_is_occupied() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied_by(utils::GROUND, utils::nav(1, 1), true);

    assert!(grid.is_occupied_by(utils::GROUND | utils::AIR, utils::nav(1, 1)));
    assert!(!grid.is_occupied_by(utils::AIR, utils::nav(1, 1)));
}

#[test]
fn set_occupied_by_updates_all_matched_layers() {
    let mut grid = utils::grid(5, 5);

    grid.set_occupied_by(utils::GROUND | utils::AIR, utils::nav(3, 3), true);

    assert!(grid.is_occupied_by(utils::GROUND, utils::nav(3, 3)));
    assert!(grid.is_occupied_by(utils::AIR, utils::nav(3, 3)));
}

#[test]
fn occupancy_reads_no_terrain() {
    let grid = utils::grid_with_water(5, 5, &[utils::nav(2, 2)]);

    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(2, 2)));
    assert!(!grid.is_statically_occupied_by(utils::GROUND, utils::nav(2, 2)));
}

//
// ─── Terrain ──────────────────────────────────────────────────────────────────
//

#[test]
fn new_grid_gives_each_cell_its_terrain() {
    let grid = utils::grid_with_water(4, 4, &[utils::nav(1, 2)]);

    assert_eq!(grid.terrain(utils::nav(1, 2)), Some(utils::WATER));
    assert_eq!(grid.terrain(utils::nav(2, 1)), Some(utils::LAND));
}

#[test]
fn off_grid_has_no_terrain() {
    let grid = utils::grid(4, 4);

    assert_eq!(grid.terrain(utils::nav(0, 4)), None);
    assert!(!grid.is_on_terrains(utils::every_terrain(), utils::nav(0, 4)));
}

#[test]
fn terrain_query_ignores_occupancy_and_claims() {
    let mut grid = utils::grid_with_water(8, 8, &[utils::nav(5, 5)]);
    grid.set_occupied_by(utils::GROUND, utils::nav(3, 3), true);
    grid.set_claimed_by(utils::GROUND, utils::nav(4, 4), true);

    assert!(grid.is_on_terrains(utils::land(), utils::nav(3, 3)));
    assert!(grid.is_on_terrains(utils::land(), utils::nav(4, 4)));
    assert!(!grid.is_on_terrains(utils::land(), utils::nav(5, 5)));
    assert!(grid.is_on_terrains(utils::every_terrain(), utils::nav(5, 5)));
}

//
// ─── Shape fits ───────────────────────────────────────────────────────────────
//

#[test]
fn cell_fits_reads_one_cell_terrain_and_layers() {
    let mut grid = utils::grid_with_water(4, 4, &[utils::nav(1, 1)]);
    grid.set_occupied_by(utils::GROUND, utils::nav(2, 2), true);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);
    let ground = LayerMask::from(utils::GROUND);

    assert!(grid.cell_fits(utils::nav(0, 0), utils::land(), ground));
    assert!(
        !grid.cell_fits(utils::nav(1, 1), utils::land(), ground),
        "water is off a land cell's terrains"
    );
    assert!(!grid.cell_fits(utils::nav(2, 2), utils::land(), ground));
    assert!(!grid.cell_fits(utils::nav(3, 3), utils::land(), ground));
    assert!(
        grid.cell_fits_statically(utils::nav(3, 3), utils::land(), ground),
        "a claim never refuses the static read"
    );
    assert!(!grid.cell_fits_statically(utils::nav(2, 2), utils::land(), ground));
}

#[test]
fn fits_honors_claims_where_fits_statically_ignores_them() {
    let mut grid = utils::grid(8, 8);
    let shape = utils::square_on_ground(2);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);

    assert!(!grid.fits(utils::nav(2, 2), shape));
    assert!(grid.fits_statically(utils::nav(2, 2), shape));
}

#[test]
fn static_occupancy_refuses_shape_on_both_queries() {
    let mut grid = utils::grid(8, 8);
    let shape = utils::square_on_ground(2);
    grid.set_occupied_by(utils::GROUND, utils::nav(3, 3), true);

    assert!(!grid.fits(utils::nav(2, 2), shape));
    assert!(!grid.fits_statically(utils::nav(2, 2), shape));
}

#[test]
fn shape_fits_only_where_every_layer_is_free() {
    let mut grid = utils::grid(8, 8);
    let tall = MoverShape::point(utils::land(), utils::GROUND | utils::AIR);
    grid.set_occupied_by(utils::AIR, utils::nav(3, 3), true);

    assert!(grid.fits(utils::nav(2, 2), tall));
    assert!(
        !grid.fits(utils::nav(3, 3), tall),
        "one occupied layer must refuse the whole shape"
    );
}

#[test]
fn cell_off_shape_terrains_refuses_it_on_both_queries() {
    let grid = utils::grid_with_water(8, 8, &[utils::nav(3, 3)]);
    let shape = utils::square_on_ground(2);

    assert!(!grid.fits(utils::nav(2, 2), shape));
    assert!(!grid.fits_statically(utils::nav(2, 2), shape));
    assert!(
        grid.fits(utils::nav(4, 4), shape),
        "a footprint clear of it still fits"
    );
}

#[test]
fn shape_over_every_terrain_fits_on_any() {
    let grid = utils::grid_with_water(8, 8, &[utils::nav(3, 3)]);

    assert!(grid.fits(utils::nav(3, 3), utils::in_air()));
}

#[test]
fn terrain_fit_ignores_claims_and_standing_footprints() {
    let mut grid = utils::grid(8, 8);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(4, 3), true);
    let size = CellSize::new(3, 2);

    assert!(!grid.fits(utils::nav(2, 2), utils::square_on_ground(2)));
    assert!(grid.fits_terrain(utils::nav(2, 2), utils::land(), size));
}

#[test]
fn terrain_fit_is_refused_by_any_cell_off_its_terrains() {
    let grid = utils::grid_with_water(8, 8, &[utils::nav(4, 3)]);

    assert!(!grid.fits_terrain(utils::nav(2, 2), utils::land(), CellSize::new(3, 2)));
    assert!(
        grid.fits_terrain(utils::nav(2, 2), utils::land(), CellSize::ONE),
        "a footprint clear of it still stands"
    );
}

#[test]
fn footprint_reaching_out_of_bounds_fits_nowhere() {
    let grid = utils::grid(8, 8);
    let shape = utils::square_on_ground(2);

    assert!(!grid.fits(utils::nav(7, 7), shape));
    assert!(!grid.fits_statically(utils::nav(7, 7), shape));
    assert!(!grid.fits_terrain(utils::nav(7, 7), utils::land(), shape.size));
}

//
// ─── Unit claims ──────────────────────────────────────────────────────────────
//

#[test]
fn claim_blocks_movement_queries_but_not_static_ones() {
    let mut grid = utils::grid(8, 8);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);

    assert!(grid.is_occupied_by(utils::GROUND, utils::nav(3, 3)));
    assert!(grid.is_claimed_by(utils::GROUND, utils::nav(3, 3)));
    assert!(!grid.is_statically_occupied_by(utils::GROUND, utils::nav(3, 3)));
}

#[test]
fn released_claim_frees_cell() {
    let mut grid = utils::grid(8, 8);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), false);

    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(3, 3)));
}

#[test]
fn static_occupancy_blocks_both_query_planes() {
    let mut grid = utils::grid(8, 8);
    grid.set_occupied_by(utils::GROUND, utils::nav(3, 3), true);

    assert!(grid.is_occupied_by(utils::GROUND, utils::nav(3, 3)));
    assert!(grid.is_statically_occupied_by(utils::GROUND, utils::nav(3, 3)));
    assert!(!grid.is_claimed_by(utils::GROUND, utils::nav(3, 3)));
}

#[test]
fn clear_claims_releases_units_but_keeps_statics() {
    let mut grid = utils::grid(8, 8);
    grid.set_claimed_by(utils::GROUND, utils::nav(2, 2), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(5, 5), true);

    grid.clear_claims();

    assert!(!grid.is_occupied_by(utils::GROUND, utils::nav(2, 2)));
    assert!(grid.is_statically_occupied_by(utils::GROUND, utils::nav(5, 5)));
}
