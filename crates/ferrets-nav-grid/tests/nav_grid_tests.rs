//! `NavGrid` terrain, layer occupancy, unit claims and shape fits.

mod utils;

use ferrets_geometry::{cell_rect::CellRect, cell_size::CellSize, projection::Projection};
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    relief::Relief,
    slope::{Direction, Slope},
};

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
#[should_panic(expected = "layer 1 is already registered")]
fn layer_registered_twice_panics() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.add_flat_layer(utils::GROUND, 1);
}

// The following tests rely on `debug_assert!` inside `assert_registered` and only
// fire in debug builds. They are excluded from release-mode test runs.

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn set_occupied_by_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.set_occupied_by(utils::AIR, utils::nav(1, 1), true);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn is_occupied_by_panics_on_unregistered_mask() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.is_occupied_by(utils::GROUND | utils::AIR, utils::nav(1, 1));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "mask contains unregistered layers")]
fn set_claimed_by_panics_on_unregistered_layer() {
    let mut grid = NavGrid::new(5, 5, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.set_claimed_by(utils::AIR, utils::nav(1, 1), true);
}

//
// ─── Occupancy ────────────────────────────────────────────────────────────────
//

#[test]
fn every_position_is_free_after_layer_registers() {
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
    let land_walker = utils::on_ground();

    assert!(grid.cell_fits(utils::nav(0, 0), land_walker));
    assert!(
        !grid.cell_fits(utils::nav(1, 1), land_walker),
        "water is off a land cell's terrains"
    );
    assert!(!grid.cell_fits(utils::nav(2, 2), land_walker));
    assert!(!grid.cell_fits(utils::nav(3, 3), land_walker));
    assert!(
        grid.cell_fits_statically(utils::nav(3, 3), land_walker),
        "a claim never refuses the static read"
    );
    assert!(!grid.cell_fits_statically(utils::nav(2, 2), land_walker));
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
    let tall = MoverShape::new(
        utils::land(),
        utils::GROUND,
        utils::GROUND | utils::AIR,
        CellSize::ONE,
        None,
    );
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
fn ground_fit_ignores_claims_and_standing_footprints() {
    let mut grid = utils::grid(8, 8);
    grid.set_claimed_by(utils::GROUND, utils::nav(3, 3), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(4, 3), true);
    let shape = MoverShape::new(
        utils::land(),
        utils::GROUND,
        utils::GROUND,
        CellSize::new(3, 2),
        None,
    );

    assert!(!grid.fits(utils::nav(2, 2), shape));
    assert!(grid.fits_ground(utils::nav(2, 2), shape));
}

#[test]
fn ground_fit_is_refused_by_any_cell_off_its_terrains() {
    let grid = utils::grid_with_water(8, 8, &[utils::nav(4, 3)]);

    assert!(!grid.fits_ground(
        utils::nav(2, 2),
        MoverShape::new(
            utils::land(),
            utils::GROUND,
            utils::GROUND,
            CellSize::new(3, 2),
            None
        )
    ));
    assert!(
        grid.fits_ground(utils::nav(2, 2), utils::square_on_ground(1)),
        "a footprint clear of it still stands"
    );
}

#[test]
fn footprint_reaching_out_of_bounds_fits_nowhere() {
    let grid = utils::grid(8, 8);
    let shape = utils::square_on_ground(2);

    assert!(!grid.fits(utils::nav(7, 7), shape));
    assert!(!grid.fits_statically(utils::nav(7, 7), shape));
    assert!(!grid.fits_ground(utils::nav(7, 7), shape));
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

//
// ─── Levels ───────────────────────────────────────────────────────────────────
//

#[test]
fn each_layer_keeps_its_own_level_at_every_cell() {
    let grid = utils::hill_grid();
    let peak = utils::nav(7, 4);

    // GROUND's peak at 3 under AIR flat at 4; AIR has no slopes, not even
    // over GROUND's ramp.
    assert_eq!(grid.cell_level(utils::GROUND, peak), Some(3));
    assert_eq!(grid.cell_level(utils::AIR, peak), Some(4));
    assert_eq!(grid.slope(utils::AIR, utils::nav(3, 1)), None);
    assert!(grid.cell_fits(peak, utils::walker()));
    assert!(grid.cell_fits(peak, utils::in_air()));
}

#[test]
fn ramp_is_climbed_only_through_its_rising_edge() {
    let grid = utils::hill_grid();

    // Up and down the ramp at (3, 1), rising east onto the plateau.
    assert!(grid.can_cross(utils::nav(3, 1), utils::nav(4, 1), utils::walker()));
    assert!(grid.can_cross(utils::nav(4, 1), utils::nav(3, 1), utils::walker()));
    // Beside it, (3, 0) at 1 meets the plateau at a cliff.
    assert!(!grid.can_cross(utils::nav(3, 0), utils::nav(4, 0), utils::walker()));
    // A ramp cell's other sides are low ground like any other.
    assert!(grid.can_cross(utils::nav(2, 1), utils::nav(3, 1), utils::walker()));

    // A ramp at (0, 1) rising east with higher ground to its north as well:
    // the north edge is a cliff it does not rise across.
    let mut grid = NavGrid::new(2, 2, |_| utils::LAND);
    grid.add_ranged_layer(utils::GROUND, 1, 2, |cell| Relief {
        level: if (cell.x, cell.y) == (0, 1) { 1 } else { 2 },
        slope: match (cell.x, cell.y) {
            (0, 1) => Some(Slope::Ramp(Direction::East)),
            _ => None,
        },
    });
    assert!(grid.can_cross(utils::nav(0, 1), utils::nav(1, 1), utils::walker()));
    assert!(!grid.can_cross(utils::nav(0, 1), utils::nav(0, 0), utils::walker()));
    assert!(grid.can_cross(utils::nav(0, 1), utils::nav(0, 0), utils::strider()));
}

#[test]
fn cliff_stops_ramp_crosser_and_cross_steps_one_level() {
    let grid = utils::hill_grid();
    // (4, 3) at 2 above (4, 4) at 1, and the plateau's (5, 0) at 2 below the
    // mountain's (6, 0) at 3.
    for (from, to) in [((4, 3), (4, 4)), ((5, 0), (6, 0))] {
        let (from, to) = (utils::nav(from.0, from.1), utils::nav(to.0, to.1));
        assert!(!grid.can_cross(from, to, utils::walker()));
        assert!(grid.can_cross(from, to, utils::strider()));
        assert!(grid.can_cross(to, from, utils::strider()));
    }
}

#[test]
fn cliff_of_two_levels_stops_cliff_walker() {
    let grid = utils::hill_grid();
    // (6, 4) at 1 beside the peak (7, 4) at 3.
    let (low, peak) = (utils::nav(6, 4), utils::nav(7, 4));

    assert!(!grid.can_cross(low, peak, utils::strider()));
}

#[test]
#[should_panic(expected = "an edge lies between cardinal neighbors")]
fn crossing_between_cells_off_one_axis_panics() {
    utils::hill_grid().can_cross(utils::nav(0, 0), utils::nav(1, 1), utils::walker());
}

#[test]
#[should_panic(expected = "a step goes to a neighboring anchor")]
fn step_beyond_neighbors_panics() {
    utils::grid(4, 4).can_step(utils::nav(0, 0), utils::nav(2, 0), utils::on_ground());
}

#[test]
#[should_panic(expected = "a step goes to a neighboring anchor")]
fn step_that_stays_put_panics() {
    utils::grid(4, 4).can_step(utils::nav(1, 1), utils::nav(1, 1), utils::on_ground());
}

#[test]
fn diagonal_cuts_ramp_top_and_never_cliff_corner() {
    let grid = utils::hill_grid();

    // From the ramp's upper cell onto the plateau below its lower one: both
    // cardinal paths cross the ramp's rising edge.
    assert!(grid.can_step(utils::nav(3, 1), utils::nav(4, 2), utils::walker()));
    // From (3, 0) to (4, 1): east first meets the cliff at (4, 0).
    assert!(!grid.can_step(utils::nav(3, 0), utils::nav(4, 1), utils::walker()));
    // From the ramp's (3, 2) to (4, 3): east then south climbs the ramp, but
    // south then east meets the cliff from (3, 3).
    assert!(!grid.can_step(utils::nav(3, 2), utils::nav(4, 3), utils::walker()));
}

#[test]
fn wide_footprint_stands_across_edges_its_cliff_crossing_crosses() {
    let grid = utils::hill_grid();
    let wide = |cliff_crossing| {
        MoverShape::new(
            utils::land(),
            utils::GROUND,
            utils::GROUND,
            CellSize::new(2, 2),
            cliff_crossing,
        )
    };

    // At (3, 1) it spans the ramp and the plateau above it.
    assert!(grid.fits(utils::nav(3, 1), wide(Some(CliffCrossing::Ramp))));
    // At (3, 0) its top row spans the cliff beside the ramp.
    assert!(!grid.fits(utils::nav(3, 0), wide(Some(CliffCrossing::Ramp))));
    assert!(grid.fits(utils::nav(3, 0), wide(Some(CliffCrossing::Cross))));
    // At (4, 3) its top row at 2 stands over its bottom row at 1.
    assert!(!grid.fits(utils::nav(4, 3), wide(Some(CliffCrossing::Ramp))));
    assert!(grid.fits(utils::nav(4, 3), wide(Some(CliffCrossing::Cross))));
}

#[test]
fn wide_footprint_steps_only_where_every_entered_cell_crosses() {
    let grid = utils::hill_grid();
    let wide = |cliff_crossing| {
        MoverShape::new(
            utils::land(),
            utils::GROUND,
            utils::GROUND,
            CellSize::new(2, 2),
            Some(cliff_crossing),
        )
    };

    // East from (2, 0): the anchor's own edge (2, 0) to (3, 0) is flat, but
    // the top row enters (4, 0) from (3, 0) across the cliff beside the ramp.
    assert!(!grid.can_step(
        utils::nav(2, 0),
        utils::nav(3, 0),
        wide(CliffCrossing::Ramp)
    ));
    assert!(grid.can_step(
        utils::nav(2, 0),
        utils::nav(3, 0),
        wide(CliffCrossing::Cross)
    ));
    // East from (2, 1): both rows enter the plateau through the ramp's
    // rising edges.
    assert!(grid.can_step(
        utils::nav(2, 1),
        utils::nav(3, 1),
        wide(CliffCrossing::Ramp)
    ));
}

#[test]
#[should_panic(expected = "a ranged layer's levels lie within 1..=2")]
fn ranged_layer_level_outside_its_range_panics() {
    let mut grid = NavGrid::new(2, 1, |_| utils::LAND);
    grid.add_ranged_layer(utils::GROUND, 1, 2, |_| Relief {
        level: 3,
        slope: None,
    });
}

#[test]
#[should_panic(expected = "layer 2 is not registered")]
fn level_of_unregistered_layer_panics() {
    let mut grid = NavGrid::new(2, 1, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.cell_level(utils::AIR, utils::nav(0, 0));
}

#[test]
fn flat_ground_is_one_level_and_no_ramp() {
    let grid = utils::hill_grid();

    assert!(grid.is_flat(utils::nav(0, 0), utils::square_on_ground(2)));
    assert!(!grid.is_flat(utils::nav(3, 1), utils::on_ground()));
    assert!(!grid.is_flat(utils::nav(4, 3), utils::square_on_ground(2)));
}

#[test]
fn relief_reads_level_and_slope_of_cell_on_layer() {
    let grid = utils::hill_grid();

    assert_eq!(
        grid.relief(utils::GROUND, utils::nav(3, 1)),
        Some(Relief {
            level: 1,
            slope: Some(Slope::Ramp(Direction::East)),
        })
    );
    assert_eq!(grid.relief(utils::GROUND, utils::nav(8, 0)), None);
}

#[test]
fn cliff_cell_takes_only_cliff_crossers_and_fliers() {
    let mut grid = NavGrid::new(4, 1, |_| utils::LAND);
    grid.add_ranged_layer(utils::GROUND, 1, 2, |cell| Relief {
        level: if cell.x >= 2 { 2 } else { 1 },
        slope: match cell.x {
            1 | 2 => Some(Slope::Cliff(Direction::East)),
            _ => None,
        },
    });
    grid.add_flat_layer(utils::AIR, 3);

    for x in [1, 2] {
        assert!(!grid.cell_fits(utils::nav(x, 0), utils::walker()));
        assert!(!grid.cell_fits_statically(utils::nav(x, 0), utils::walker()));
        assert!(grid.cell_fits(utils::nav(x, 0), utils::strider()));
        assert!(grid.cell_fits(utils::nav(x, 0), utils::in_air()));
        assert!(!grid.fits_ground(utils::nav(x, 0), utils::walker()));
        assert!(grid.fits_ground(utils::nav(x, 0), utils::strider()));
    }
    assert!(grid.cell_fits(utils::nav(0, 0), utils::walker()));
}

#[test]
fn touch_reaches_only_across_edge_it_could_step() {
    let grid = utils::hill_grid();
    let reaches = |from: (u32, u32), to: (u32, u32), range, shape| {
        grid.reaches(
            Projection::Isometric,
            CellRect::new(utils::nav(from.0, from.1), CellSize::ONE),
            shape,
            CellRect::new(utils::nav(to.0, to.1), CellSize::ONE),
            range,
        )
    };

    // Below the cliff at (3, 0), the plateau's (4, 0) is a touch away but
    // out of reach; over the ramp's edge it is not.
    assert!(!reaches((3, 0), (4, 0), 1, utils::walker()));
    assert!(reaches((3, 1), (4, 1), 1, utils::walker()));
    assert!(reaches((3, 0), (4, 0), 1, utils::strider()));
    // A longer reach ignores the cliff.
    assert!(reaches((3, 0), (4, 0), 2, utils::walker()));
    // A diagonal touch needs both paths around it: from the ramp's (3, 2),
    // (4, 3) is a cliff away by (3, 3).
    assert!(!reaches((3, 2), (4, 3), 1, utils::walker()));
}
