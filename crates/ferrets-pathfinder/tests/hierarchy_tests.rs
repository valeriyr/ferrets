//! The [`NavHierarchy`]: cluster geometry, entrance transition extraction,
//! and flood-filled connectivity regions.

mod utils;

use ferrets_pathfinder::{
    hierarchy::{ClusterPos, NavHierarchy, Transition},
    layer_mask::LayerMask,
};

//
// ─── Cluster geometry ─────────────────────────────────────────────────────────
//

#[test]
fn cluster_of_maps_cells_to_clusters() {
    let grid = utils::grid(8, 12);
    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.cluster_of(utils::nav(0, 0)),
        ClusterPos { x: 0, y: 0 }
    );
    assert_eq!(
        hierarchy.cluster_of(utils::nav(3, 3)),
        ClusterPos { x: 0, y: 0 }
    );
    assert_eq!(
        hierarchy.cluster_of(utils::nav(4, 3)),
        ClusterPos { x: 1, y: 0 }
    );
    assert_eq!(
        hierarchy.cluster_of(utils::nav(7, 11)),
        ClusterPos { x: 1, y: 2 }
    );
}

#[test]
#[should_panic(expected = "cluster size must be greater than 0")]
fn zero_cluster_size_panics() {
    let grid = utils::grid(8, 8);
    utils::build_ground(&grid, 0);
}

#[test]
fn cluster_touches_itself_and_its_eight_neighbors() {
    let center = ClusterPos { x: 2, y: 2 };

    assert!(center.touches(center));
    for x in 1..=3 {
        for y in 1..=3 {
            assert!(center.touches(ClusterPos { x, y }));
        }
    }
    assert!(!center.touches(ClusterPos { x: 4, y: 2 }));
    assert!(!center.touches(ClusterPos { x: 0, y: 4 }));
}

//
// ─── Entrance extraction ──────────────────────────────────────────────────────
//

#[test]
fn narrow_entrance_yields_middle_transition() {
    // Two 4×4 clusters side by side, fully open. The border run is 4 cells
    // tall — narrower than a wide entrance — so one transition sits at its
    // middle.
    let grid = utils::grid(8, 4);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
}

#[test]
fn wide_entrance_yields_transitions_at_both_ends() {
    // Two 8×8 clusters side by side, fully open. The border run is 8 cells
    // tall — wide — so transitions sit at both ends.
    let grid = utils::grid(16, 8);

    let hierarchy = utils::build_ground(&grid, 8);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(7, 0, 8, 0), transition(7, 7, 8, 7)]
    );
}

#[test]
fn wall_splits_border_into_separate_entrances() {
    // . . . X|. . . .   y=0   the border column pair is x=3 | x=4;
    // . . . .|X . . .   y=1   X at (3,0) kills the y=0 pair and X at (4,1)
    // . . . .|. . . .   y=2   kills the y=1 pair, leaving one 2-cell run at
    // . . . .|. . . .   y=3   y=2..=3 whose middle transition lands on y=3.
    let mut grid = utils::grid(8, 4);
    grid.set_occupied(utils::GROUND, utils::nav(3, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(4, 1), true);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 3, 4, 3)]
    );
}

#[test]
fn blocked_border_yields_no_transitions() {
    // A full wall down the border column leaves the two clusters unconnected.
    let mut grid = utils::grid(8, 4);
    for y in 0..4 {
        grid.set_occupied(utils::GROUND, utils::nav(4, y), true);
    }

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.transitions(utils::GROUND).count(), 0);
}

#[test]
fn horizontal_border_pairs_upper_and_lower_cells() {
    // Two 4×4 clusters stacked, fully open: one narrow entrance across the
    // horizontal border, `a` above, `b` below.
    let grid = utils::grid(4, 8);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(2, 3, 2, 4)]
    );
}

#[test]
fn edge_clusters_shorter_than_cluster_size_still_extract() {
    // 6×4 grid with 4-cell clusters: the right cluster is only 2 cells wide,
    // but the border at x=3|4 still yields its middle transition.
    let grid = utils::grid(6, 4);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
}

//
// ─── Regions ──────────────────────────────────────────────────────────────────
//

#[test]
fn open_grid_is_one_region() {
    let grid = utils::grid(8, 8);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::GROUND), 1);
    assert!(hierarchy.same_region(utils::GROUND, utils::nav(0, 0), utils::nav(7, 7)));
}

#[test]
fn wall_splits_grid_into_two_regions() {
    // A full vertical wall at x=4: the halves are separate regions, and the
    // wall cells themselves have none.
    let mut grid = utils::grid(8, 8);
    for y in 0..8 {
        grid.set_occupied(utils::GROUND, utils::nav(4, y), true);
    }

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::GROUND), 2);
    assert!(!hierarchy.same_region(utils::GROUND, utils::nav(0, 0), utils::nav(7, 7)));
    assert!(hierarchy.same_region(utils::GROUND, utils::nav(0, 0), utils::nav(3, 7)));
    assert_eq!(hierarchy.region_of(utils::GROUND, utils::nav(4, 3)), None);
}

#[test]
fn corner_touching_areas_are_separate_regions() {
    // X S   y=0   S = (1,0) and G = (0,1) touch only diagonally through a
    // G X   y=1   cut corner, which movement cannot cross either.
    let mut grid = utils::grid(2, 2);
    grid.set_occupied(utils::GROUND, utils::nav(0, 0), true);
    grid.set_occupied(utils::GROUND, utils::nav(1, 1), true);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::GROUND), 2);
    assert!(!hierarchy.same_region(utils::GROUND, utils::nav(1, 0), utils::nav(0, 1)));
}

#[test]
fn out_of_bounds_cell_has_no_region() {
    let grid = utils::grid(4, 4);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_of(utils::GROUND, utils::nav(4, 0)), None);
    assert_eq!(hierarchy.region_of(utils::GROUND, utils::nav(0, 4)), None);
}

#[test]
fn masks_have_independent_regions() {
    // A wall on GROUND only: GROUND splits in two, AIR stays whole, and the
    // combined mask follows the intersection of what both layers pass.
    let mut grid = utils::grid(8, 4);
    for y in 0..4 {
        grid.set_occupied(utils::GROUND, utils::nav(4, y), true);
    }

    let hierarchy = NavHierarchy::build(
        &grid,
        4,
        &[
            LayerMask::from(utils::GROUND),
            LayerMask::from(utils::AIR),
            utils::GROUND | utils::AIR,
        ],
    );

    assert_eq!(hierarchy.region_count(utils::GROUND), 2);
    assert_eq!(hierarchy.region_count(utils::AIR), 1);
    assert_eq!(hierarchy.region_count(utils::GROUND | utils::AIR), 2);
}

//
// ─── Dirty marking and refresh ────────────────────────────────────────────────
//

#[test]
fn refresh_re_derives_marked_borders() {
    // Blocking the middle border pair splits the single entrance in two once
    // the cells are marked and the hierarchy refreshed.
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    grid.set_occupied(utils::GROUND, utils::nav(3, 2), true);
    grid.set_occupied(utils::GROUND, utils::nav(4, 2), true);
    hierarchy.mark_dirty(utils::nav(3, 2));
    hierarchy.mark_dirty(utils::nav(4, 2));
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 1, 4, 1), transition(3, 3, 4, 3)]
    );
    assert_eq!(hierarchy.region_of(utils::GROUND, utils::nav(3, 2)), None);
}

#[test]
fn refresh_restores_entrance_when_footprint_clears() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    grid.set_occupied(utils::GROUND, utils::nav(4, 2), true);
    hierarchy.mark_dirty(utils::nav(4, 2));
    hierarchy.refresh(&grid);

    grid.set_occupied(utils::GROUND, utils::nav(4, 2), false);
    hierarchy.mark_dirty(utils::nav(4, 2));
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
    assert_eq!(hierarchy.region_count(utils::GROUND), 1);
}

#[test]
fn unmarked_changes_stay_stale_until_marked() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    for y in 0..4 {
        grid.set_occupied(utils::GROUND, utils::nav(4, y), true);
    }

    // Refresh without any mark: the hierarchy must not chase the grid.
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy.transitions(utils::GROUND).count(), 1);
    assert_eq!(hierarchy.region_count(utils::GROUND), 1);

    // One marked cell of the wall re-derives its whole border and refloods.
    hierarchy.mark_dirty(utils::nav(4, 0));
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy.transitions(utils::GROUND).count(), 0);
    assert_eq!(hierarchy.region_count(utils::GROUND), 2);
}

#[test]
fn unit_claims_are_invisible_to_hierarchy() {
    // A wall of unit claims down the border column: even marked and
    // refreshed, the hierarchy sees the static plane only.
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    for y in 0..4 {
        grid.set_claimed_by(utils::GROUND, utils::nav(4, y), true);
        hierarchy.mark_dirty(utils::nav(4, y));
    }
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy.transitions(utils::GROUND).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
    assert_eq!(hierarchy.region_count(utils::GROUND), 1);
}

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

#[test]
fn rebuilding_from_same_grid_yields_identical_hierarchy() {
    let grid = utils::hollow_ring_grid(32, utils::nav(16, 16), 6);

    let first = utils::build_ground(&grid, 8);
    let second = utils::build_ground(&grid, 8);

    assert_eq!(first, second);
}

//
// ─── Unknown masks ────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "no hierarchy built for mask")]
fn querying_unknown_mask_panics() {
    let grid = utils::grid(4, 4);
    let hierarchy = utils::build_ground(&grid, 4);

    hierarchy.region_of(utils::AIR, utils::nav(0, 0));
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

fn transition(ax: u32, ay: u32, bx: u32, by: u32) -> Transition {
    Transition {
        a: utils::nav(ax, ay),
        b: utils::nav(bx, by),
    }
}
