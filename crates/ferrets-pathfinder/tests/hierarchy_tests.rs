//! The [`NavHierarchy`]: cluster geometry, entrance transition extraction,
//! and flood-filled connectivity regions.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_nav_grid::{
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    relief::Relief,
    slope::{Direction, Slope},
};
use ferrets_pathfinder::hierarchy::{ClusterPos, NavHierarchy, Transition};

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
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
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
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(7, 0, 8, 0), transition(7, 7, 8, 7)]
    );
}

#[test]
fn blocked_border_pairs_shrink_entrance_to_remaining_run() {
    // . . . X|. . . .   y=0   the border column pair is x=3 | x=4;
    // . . . .|X . . .   y=1   X at (3,0) kills the y=0 pair and X at (4,1)
    // . . . .|. . . .   y=2   kills the y=1 pair, leaving one 2-cell run at
    // . . . .|. . . .   y=3   y=2..=3 whose middle transition lands on y=3.
    let mut grid = utils::grid(8, 4);
    grid.set_occupied_by(utils::GROUND, utils::nav(3, 0), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(4, 1), true);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(3, 3, 4, 3)]
    );
}

#[test]
fn blocked_border_yields_no_transitions() {
    // A full wall down the border column leaves the two clusters unconnected.
    let mut grid = utils::grid(8, 4);
    for y in 0..4 {
        grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
    }

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.transitions(utils::on_ground()).count(), 0);
}

#[test]
fn horizontal_border_pairs_upper_and_lower_cells() {
    // Two 4×4 clusters stacked, fully open: one narrow entrance across the
    // horizontal border, `a` above, `b` below.
    let grid = utils::grid(4, 8);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
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
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
}

#[test]
fn wide_shape_entrance_middle_respects_footprint_reach() {
    // The same fully open 4-tall border as the narrow case, seen by a 2x2:
    // anchors at y=3 would hang off the south edge, so the run is y=0..=2 and
    // its middle lands one cell higher than the narrow shape's.
    let grid = utils::grid(8, 4);
    let narrow = utils::on_ground();
    let wide = utils::square_on_ground(2);

    let hierarchy = utils::build_for(&grid, 4, &[narrow, wide]);

    assert_eq!(
        hierarchy.transitions(narrow).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
    assert_eq!(
        hierarchy.transitions(wide).collect::<Vec<_>>(),
        vec![transition(3, 1, 4, 1)]
    );
}

#[test]
fn entrance_across_cliff_lies_only_at_its_ramp() {
    // Two 4×4 clusters side by side, the east one a plateau one level up,
    // and a ramp at (3, 0) rising onto it: a walker's only way across the
    // border is the ramp's row, so its one transition sits there rather than
    // at the border's middle.
    let mut grid = NavGrid::new(8, 4, |_| utils::LAND);
    grid.add_ranged_layer(utils::GROUND, 1, 2, |cell| Relief {
        level: if cell.x >= 4 { 2 } else { 1 },
        slope: match (cell.x, cell.y) {
            (3, 0) => Some(Slope::Ramp(Direction::East)),
            _ => None,
        },
    });

    let hierarchy = utils::build_for(&grid, 4, &[utils::walker()]);

    assert_eq!(
        hierarchy.transitions(utils::walker()).collect::<Vec<_>>(),
        vec![transition(3, 0, 4, 0)]
    );
}

#[test]
fn run_across_cliff_gets_entrance_on_each_level() {
    // Clusters of 8, a cliff between rows 3 and 4 inside them. The gap at
    // x=7 spans rows 2 to 5 of the 7|8 border: rows 2 and 3 join the lower
    // local regions on either side, rows 4 and 5 the upper ones, so each pair
    // gets its own entrance, the middle of its two-row run. The open 15|16
    // border likewise: rows 0 to 3 and rows 4 to 7, each a four-row run
    // entered at its middle.
    let grid = utils::terraced_gap_grid();
    let hierarchy = utils::build_for(&grid, 8, &[utils::walker()]);

    assert_eq!(
        hierarchy.transitions(utils::walker()).collect::<Vec<_>>(),
        vec![
            transition(7, 3, 8, 3),
            transition(7, 5, 8, 5),
            transition(15, 2, 16, 2),
            transition(15, 6, 16, 6)
        ]
    );
}

//
// ─── Regions ──────────────────────────────────────────────────────────────────
//

#[test]
fn open_grid_is_one_region() {
    let grid = utils::grid(8, 8);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 1);
    assert!(hierarchy.same_region(utils::nav(0, 0), utils::on_ground(), utils::nav(7, 7)));
}

#[test]
fn wall_splits_grid_into_two_regions() {
    // A full vertical wall at x=4: the halves are separate regions, and the
    // wall cells themselves have none.
    let mut grid = utils::grid(8, 8);
    for y in 0..8 {
        grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
    }

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert!(!hierarchy.same_region(utils::nav(0, 0), utils::on_ground(), utils::nav(7, 7)));
    assert!(hierarchy.same_region(utils::nav(0, 0), utils::on_ground(), utils::nav(3, 7)));
    assert_eq!(
        hierarchy.region_of(utils::nav(4, 3), utils::on_ground()),
        None
    );
}

#[test]
fn corner_touching_areas_are_separate_regions() {
    // X S   y=0   S = (1,0) and G = (0,1) touch only diagonally through a
    // G X   y=1   cut corner, which movement cannot cross either.
    let mut grid = utils::grid(2, 2);
    grid.set_occupied_by(utils::GROUND, utils::nav(0, 0), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(1, 1), true);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert!(!hierarchy.same_region(utils::nav(1, 0), utils::on_ground(), utils::nav(0, 1)));
}

#[test]
fn out_of_bounds_cell_has_no_region() {
    let grid = utils::grid(4, 4);

    let hierarchy = utils::build_ground(&grid, 4);

    assert_eq!(
        hierarchy.region_of(utils::nav(4, 0), utils::on_ground()),
        None
    );
    assert_eq!(
        hierarchy.region_of(utils::nav(0, 4), utils::on_ground()),
        None
    );
}

#[test]
fn masks_have_independent_regions() {
    // A wall on GROUND only: GROUND splits in two, AIR stays whole, and a
    // shape occupying both layers splits where either is occupied.
    let mut grid = utils::grid(8, 4);
    for y in 0..4 {
        grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
    }

    let both = MoverShape::new(
        utils::land(),
        utils::GROUND,
        utils::GROUND | utils::AIR,
        CellSize::ONE,
        None,
    );
    let hierarchy = NavHierarchy::build(&grid, 4, &[utils::on_ground(), utils::in_air(), both]);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert_eq!(hierarchy.region_count(utils::in_air()), 1);
    assert_eq!(hierarchy.region_count(both), 2);
}

#[test]
fn terrains_have_independent_regions() {
    // A strip of water across the middle: a land shape is split in two, a
    // shape over every terrain on the same layer is not.
    let strip: Vec<_> = (0..4).map(|y| utils::nav(4, y)).collect();
    let grid = utils::grid_with_water(8, 4, &strip);
    let wading = MoverShape::point(utils::every_terrain(), utils::GROUND, None);

    let hierarchy = NavHierarchy::build(&grid, 4, &[utils::on_ground(), wading]);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert_eq!(hierarchy.region_count(wading), 1);
}

#[test]
fn cliff_splits_regions_ramp_joins() {
    let grid = utils::hill_grid();
    let (walker, strider) = (utils::walker(), utils::strider());
    let hierarchy = utils::build_for(&grid, 4, &[walker, strider]);

    // Low ground and the plateau meet over the ramp; the mountain at 3 has
    // none, so it is its own region for a walker and not for a cliff walker.
    assert!(hierarchy.same_region(utils::nav(0, 0), walker, utils::nav(5, 3)));
    assert!(!hierarchy.same_region(utils::nav(5, 0), walker, utils::nav(6, 0)));
    assert!(hierarchy.same_region(utils::nav(5, 0), strider, utils::nav(6, 0)));
}

//
// ─── Clearance ────────────────────────────────────────────────────────────────
//

#[test]
fn wide_mover_does_not_connect_through_narrow_gap() {
    // A wall down x=4 with a single-cell hole at y=2:
    //
    //   . . . . X . . .
    //   . . . . X . . .
    //   . . . .   . . .   <- the one-cell gap
    //   . . . . X . . .
    //
    // A single-cell mover crosses it, a two-cell one cannot fit and so its
    // halves are genuinely separate regions — the whole point of clearance.
    let mut grid = utils::grid(8, 8);
    for y in 0..8 {
        if y != 2 {
            grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
        }
    }

    let narrow = utils::on_ground();
    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 4, &[narrow, wide]);

    assert_eq!(hierarchy.region_count(narrow), 1);
    assert!(hierarchy.same_region(utils::nav(0, 2), narrow, utils::nav(6, 2)));

    assert_eq!(
        hierarchy.region_count(wide),
        2,
        "a two-cell mover squeezed through a one-cell gap"
    );
    assert!(!hierarchy.same_region(utils::nav(0, 2), wide, utils::nav(6, 2)));
}

#[test]
fn wide_mover_connects_through_gap_it_fits() {
    // The same wall with a two-cell hole at y=2..=3 lets both through.
    let mut grid = utils::grid(8, 8);
    for y in 0..8 {
        if y != 2 && y != 3 {
            grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
        }
    }

    let wide = utils::square_on_ground(2);
    let hierarchy = utils::build_for(&grid, 4, &[wide]);

    assert_eq!(hierarchy.region_count(wide), 1);
    assert!(hierarchy.same_region(utils::nav(0, 2), wide, utils::nav(6, 2)));
}

#[test]
fn wide_mover_has_no_region_where_its_footprint_would_not_fit() {
    // A cell one step from a wall is a legal anchor for a single-cell mover and
    // not for a wider one, whose footprint would reach into the wall.
    let mut grid = utils::grid(8, 8);
    for y in 0..8 {
        grid.set_occupied_by(utils::GROUND, utils::nav(5, y), true);
    }

    let hierarchy = utils::build_for(&grid, 4, &[utils::on_ground(), utils::square_on_ground(2)]);

    // Region ids are minted in flood order, row-major from the origin, so
    // the left half is region 0 — pinned, because peers replay the flood.
    assert_eq!(
        hierarchy.region_of(utils::nav(4, 0), utils::on_ground()),
        Some(0)
    );
    assert_eq!(
        hierarchy.region_of(utils::nav(4, 0), utils::square_on_ground(2)),
        None,
        "anchoring a two-cell footprint at x=4 reaches into the wall at x=5"
    );
}

//
// ─── Dirty marking and refresh ────────────────────────────────────────────────
//

#[test]
fn refresh_re_derives_borders_of_marked_cells() {
    // Blocking the middle border pair splits the single entrance in two once
    // the cells are marked and the hierarchy refreshed.
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    grid.set_occupied_by(utils::GROUND, utils::nav(3, 2), true);
    grid.set_occupied_by(utils::GROUND, utils::nav(4, 2), true);
    hierarchy.mark_dirty(utils::nav(3, 2), utils::GROUND);
    hierarchy.mark_dirty(utils::nav(4, 2), utils::GROUND);
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(3, 1, 4, 1), transition(3, 3, 4, 3)]
    );
    assert_eq!(
        hierarchy.region_of(utils::nav(3, 2), utils::on_ground()),
        None
    );
}

#[test]
fn hierarchy_is_dirty_from_marked_change_until_refreshed() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);
    assert!(!hierarchy.is_dirty());

    grid.set_occupied_by(utils::GROUND, utils::nav(3, 2), true);
    hierarchy.mark_dirty(utils::nav(3, 2), utils::GROUND);
    assert!(hierarchy.is_dirty());

    hierarchy.refresh(&grid);
    assert!(!hierarchy.is_dirty());
}

#[test]
fn refresh_restores_entrance_when_footprint_clears() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    grid.set_occupied_by(utils::GROUND, utils::nav(4, 2), true);
    hierarchy.mark_dirty(utils::nav(4, 2), utils::GROUND);
    hierarchy.refresh(&grid);

    grid.set_occupied_by(utils::GROUND, utils::nav(4, 2), false);
    hierarchy.mark_dirty(utils::nav(4, 2), utils::GROUND);
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
    assert_eq!(hierarchy.region_count(utils::on_ground()), 1);
}

#[test]
fn unmarked_changes_stay_stale_until_marked() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    for y in 0..4 {
        grid.set_occupied_by(utils::GROUND, utils::nav(4, y), true);
    }

    // Refresh without any mark: the hierarchy must not chase the grid.
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy.transitions(utils::on_ground()).count(), 1);
    assert_eq!(hierarchy.region_count(utils::on_ground()), 1);

    // One marked cell of the wall re-derives its whole border and refloods.
    hierarchy.mark_dirty(utils::nav(4, 0), utils::GROUND);
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy.transitions(utils::on_ground()).count(), 0);
    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
}

#[test]
fn unit_claims_are_invisible_to_hierarchy() {
    // A wall of unit claims down the border column: even marked and
    // refreshed, the hierarchy sees the static plane only.
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    for y in 0..4 {
        grid.set_claimed_by(utils::GROUND, utils::nav(4, y), true);
        hierarchy.mark_dirty(utils::nav(4, y), utils::GROUND);
    }
    hierarchy.refresh(&grid);

    assert_eq!(
        hierarchy
            .transitions(utils::on_ground())
            .collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
    assert_eq!(hierarchy.region_count(utils::on_ground()), 1);
}

#[test]
fn refresh_re_derives_wide_transitions_from_changes_past_border() {
    // 8×4, cluster 4: the wide shape's only entrance over the x=3|4 border sits
    // at its middle. A wall at x=5 lies on no border, but a 2×2 anchored at
    // (4,0) or (4,1) covers it — so the entrance must re-derive, or the
    // hierarchy keeps offering crossings the footprint no longer fits.
    let mut grid = utils::grid(8, 4);
    let wide = utils::square_on_ground(2);
    let mut hierarchy = utils::build_for(&grid, 4, &[wide]);

    assert_eq!(
        hierarchy.transitions(wide).collect::<Vec<_>>(),
        vec![transition(3, 1, 4, 1)]
    );

    grid.set_occupied_by(utils::GROUND, utils::nav(5, 1), true);
    hierarchy.mark_dirty(utils::nav(5, 1), utils::GROUND);
    hierarchy.refresh(&grid);

    // Anchors (4,0) and (4,1) now reach into the wall; only y=2 pairs survive
    // (y=3 anchors always ran off the grid's south edge).
    assert_eq!(
        hierarchy.transitions(wide).collect::<Vec<_>>(),
        vec![transition(3, 2, 4, 2)]
    );
}

#[test]
fn refresh_after_cut_equals_fresh_build() {
    // 12×12, clusters of 4: a wall down column 4 splits the map. The 2×2
    // shape's anchors in column 3, a cluster to the left, reach it, so that
    // cluster refloods too; marked and refreshed, the hierarchy is the one a
    // fresh build of the walled grid makes, two regions for each shape.
    let mut grid = utils::grid(12, 12);
    let shapes = [utils::on_ground(), utils::square_on_ground(2)];
    let mut hierarchy = utils::build_for(&grid, 4, &shapes);

    for y in 0..12 {
        set_blocked(&mut grid, &mut hierarchy, utils::nav(4, y), true);
    }
    hierarchy.refresh(&grid);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert_eq!(hierarchy.region_count(utils::square_on_ground(2)), 2);
    assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));
}

#[test]
fn refresh_after_cut_inside_cluster_equals_fresh_build() {
    // A wall down column 5, inside the clusters spanning columns 4 to 7 and
    // on none of their borders: it splits their local regions in two, and
    // the right-hand ones, renumbered, must re-link across the border at
    // 7|8 though no cell of that border changed.
    let mut grid = utils::grid(12, 12);
    let shapes = [utils::on_ground(), utils::square_on_ground(2)];
    let mut hierarchy = utils::build_for(&grid, 4, &shapes);

    for y in 0..12 {
        set_blocked(&mut grid, &mut hierarchy, utils::nav(5, y), true);
    }
    hierarchy.refresh(&grid);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 2);
    assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));
}

#[test]
fn refresh_after_gap_equals_fresh_build() {
    // The same wall, then (4, 7) cleared again: the one-cell shape joins the
    // halves through the gap; the 2×2 shape does not fit through it and keeps
    // its two regions.
    let mut grid = utils::grid(12, 12);
    let shapes = [utils::on_ground(), utils::square_on_ground(2)];
    let mut hierarchy = utils::build_for(&grid, 4, &shapes);
    for y in 0..12 {
        set_blocked(&mut grid, &mut hierarchy, utils::nav(4, y), true);
    }
    hierarchy.refresh(&grid);

    set_blocked(&mut grid, &mut hierarchy, utils::nav(4, 7), false);
    hierarchy.refresh(&grid);

    assert_eq!(hierarchy.region_count(utils::on_ground()), 1);
    assert_eq!(hierarchy.region_count(utils::square_on_ground(2)), 2);
    assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));
}

#[test]
fn refresh_after_changes_at_cluster_corner_equals_fresh_build() {
    // The four cells around the corner where clusters (0, 0), (1, 0), (0, 1)
    // and (1, 1) meet, blocked and then two of them cleared again.
    let mut grid = utils::grid(12, 12);
    let shapes = [utils::on_ground(), utils::square_on_ground(2)];
    let mut hierarchy = utils::build_for(&grid, 4, &shapes);
    let corner = [
        utils::nav(3, 3),
        utils::nav(4, 3),
        utils::nav(3, 4),
        utils::nav(4, 4),
    ];
    for cell in corner {
        set_blocked(&mut grid, &mut hierarchy, cell, true);
    }
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));

    set_blocked(&mut grid, &mut hierarchy, utils::nav(4, 3), false);
    set_blocked(&mut grid, &mut hierarchy, utils::nav(3, 4), false);
    hierarchy.refresh(&grid);
    assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));
}

#[test]
fn refresh_after_change_on_either_side_of_border_equals_fresh_build() {
    // 12×12, clusters of 4, the one-cell shape alone: a cell left of the
    // border at 3|4, one above and one below a horizontal border, each moving
    // its border's middle transition.
    let mut grid = utils::grid(12, 12);
    let shapes = [utils::on_ground()];
    let mut hierarchy = utils::build_for(&grid, 4, &shapes);

    for cell in [utils::nav(3, 6), utils::nav(6, 3), utils::nav(10, 4)] {
        set_blocked(&mut grid, &mut hierarchy, cell, true);
        hierarchy.refresh(&grid);
        assert_eq!(hierarchy, utils::build_for(&grid, 4, &shapes));
    }
}

#[test]
fn ground_write_dirties_ground_shape_alone() {
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_for(&grid, 4, &[utils::on_ground(), utils::in_air()]);

    grid.set_occupied_by(utils::GROUND, utils::nav(3, 2), true);
    hierarchy.mark_dirty(utils::nav(3, 2), utils::GROUND);
    assert!(hierarchy.is_dirty_for(utils::on_ground()));
    assert!(!hierarchy.is_dirty_for(utils::in_air()));

    hierarchy.refresh(&grid);
    assert!(!hierarchy.is_dirty_for(utils::on_ground()));
}

#[test]
#[should_panic(expected = "a static change lies on the grid")]
fn marking_cell_off_grid_panics() {
    // 10×4, clusters of 4: (10, 0) lies past the grid's edge but inside the
    // far-edge cluster's full square.
    let grid = utils::grid(10, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    hierarchy.mark_dirty(utils::nav(10, 0), utils::GROUND);
}

#[test]
#[should_panic(expected = "a static change lies on some layer")]
fn marking_change_on_no_layer_panics() {
    let grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_ground(&grid, 4);

    hierarchy.mark_dirty(utils::nav(3, 2), LayerMask::EMPTY);
}

#[test]
fn write_on_layer_shape_need_not_be_free_on_leaves_it_clean() {
    // An air shape is free of what stands on the ground: a ground write
    // queues nothing for it, and the hierarchy, serving it alone, reads clean.
    let mut grid = utils::grid(8, 4);
    let mut hierarchy = utils::build_for(&grid, 4, &[utils::in_air()]);

    grid.set_occupied_by(utils::GROUND, utils::nav(4, 2), true);
    hierarchy.mark_dirty(utils::nav(4, 2), utils::GROUND);

    assert!(!hierarchy.is_dirty());
}

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

#[test]
fn rebuilding_from_same_grid_yields_identical_hierarchy() {
    let grid = utils::hollow_ring_grid(32, utils::nav(16, 16), 6);
    let shapes = [utils::on_ground(), utils::square_on_ground(2)];

    let first = utils::build_for(&grid, 8, &shapes);
    let second = utils::build_for(&grid, 8, &shapes);

    assert_eq!(first, second);
}

//
// ─── Unserved shapes ──────────────────────────────────────────────────────────
//

#[test]
#[should_panic(
    expected = "no hierarchy built for shape on terrains 0b11 and layers 0b10 at 1x1, cliff crossing None"
)]
fn querying_unserved_layer_panics() {
    let grid = utils::grid(4, 4);
    let hierarchy = utils::build_ground(&grid, 4);

    hierarchy.region_of(utils::nav(0, 0), utils::in_air());
}

#[test]
#[should_panic(
    expected = "no hierarchy built for shape on terrains 0b1 and layers 0b1 at 2x2, cliff crossing None"
)]
fn querying_unserved_footprint_panics() {
    // Same layer, wider footprint: a distinct abstraction, because a wide mover
    // genuinely has fewer ways through the map than a narrow one.
    let grid = utils::grid(8, 8);
    let hierarchy = utils::build_ground(&grid, 4);

    hierarchy.region_of(utils::nav(0, 0), utils::square_on_ground(2));
}

#[test]
#[should_panic(
    expected = "no hierarchy built for shape on terrains 0b11 and layers 0b1 at 1x1, cliff crossing None"
)]
fn querying_unserved_terrains_panics() {
    // Same layer and footprint, more terrains: a distinct abstraction, because
    // a mover that also takes water has other ways through the map.
    let grid = utils::grid(4, 4);
    let hierarchy = utils::build_ground(&grid, 4);

    hierarchy.region_of(
        utils::nav(0, 0),
        MoverShape::point(utils::every_terrain(), utils::GROUND, None),
    );
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

/// Sets `cell` blocked or free on the GROUND layer and marks it for the
/// hierarchy.
fn set_blocked(grid: &mut NavGrid, hierarchy: &mut NavHierarchy, cell: CellPos, blocked: bool) {
    grid.set_occupied_by(utils::GROUND, cell, blocked);
    hierarchy.mark_dirty(cell, utils::GROUND);
}
