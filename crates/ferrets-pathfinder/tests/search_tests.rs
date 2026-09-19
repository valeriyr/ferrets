//! Grid search: finding free positions around cells and footprints.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_pathfinder::{
    nav_grid::NavGrid,
    search::{self, Expansion, Footing},
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

    assert!(
        result.is_none(),
        "a search that will not cross blocked ground never reaches past the ring"
    );
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

    assert!(
        result.is_none(),
        "crossing blocked ground still needs somewhere free to stop"
    );
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
    // The anchor rectangle itself is scanned first; on an empty grid every
    // cell of a 2×2 lies the same distance from its middle, so the tie goes
    // to the first row and column.
    let grid = utils::grid(10, 10);

    for footing in [Footing::Free, Footing::Shared] {
        let found = search::find_placement_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::new(2, 2)),
            CellSize::ONE,
            8,
            footing,
        );

        assert_eq!(
            found,
            Some(utils::nav(3, 3)),
            "on empty ground both footings take the anchor cell"
        );
    }
}

#[test]
fn placement_skips_occupied_anchor_and_picks_nearest_ring_cell() {
    // . . . . . .   y=2   anchor 2×2 at (3,3) fully occupied → ring 1 spans
    // . . A A . .   y=3   (2,2)..(5,5); the free cell nearest the anchor's
    // . . A A . .   y=4   middle is (3,2), not the corner (2,2).
    let mut grid = utils::grid(10, 10);
    for y in 3..5 {
        for x in 3..5 {
            grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
        }
    }

    let asked = |footing| {
        search::find_placement_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::new(2, 2)),
            CellSize::ONE,
            8,
            footing,
        )
    };

    assert_eq!(asked(Footing::Free), Some(utils::nav(3, 2)));
    assert_eq!(
        asked(Footing::Shared),
        Some(utils::nav(3, 3)),
        "a shared footing has no reason to skip the anchor others hold"
    );
}

#[test]
fn placement_requires_whole_spawn_footprint() {
    // Single free cells are not enough for a 2×2 spawn: the position nearest
    // the middle whose whole 2×2 footprint is free wins.
    let mut grid = utils::grid(8, 8);
    // Occupy the anchor cell and checker the first ring so no 2×2 fits there.
    grid.set_occupied(utils::GROUND, utils::nav(3, 3), true);
    for &(x, y) in &[(2, 2), (4, 2), (2, 4), (4, 4)] {
        grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
    }

    let asked = |footing| {
        search::find_placement_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::ONE),
            CellSize::new(2, 2),
            8,
            footing,
        )
    };

    let origin = asked(Footing::Free).expect("a 2×2 area exists within the radius");
    for dy in 0..2 {
        for dx in 0..2 {
            assert!(
                grid.is_passable_by(utils::GROUND, utils::nav(origin.x + dx, origin.y + dy)),
                "cell ({dx}, {dy}) of the area found is held by something"
            );
        }
    }
    assert_eq!(
        asked(Footing::Shared),
        Some(utils::nav(3, 3)),
        "a shared footing needs the whole 2×2 too, but only from the terrain"
    );
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

    let asked = |footing| {
        search::find_placement_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::new(2, 2)),
            CellSize::ONE,
            1,
            footing,
        )
    };
    assert_eq!(
        asked(Footing::Shared),
        Some(utils::nav(3, 3)),
        "a shared footing gives up on nothing here: held ground is ground enough"
    );
    let found = asked(Footing::Free);

    assert_eq!(
        found, None,
        "a free footing wants ground nothing stands on, and there is none"
    );
}

//
// ─── find_placements_near ─────────────────────────────────────────────────────
//

#[test]
fn placements_come_out_in_scan_order() {
    // Rings outward from the anchor, and within a ring the ground nearest the
    // anchor's middle first, ties by row then column — which is what makes
    // the answer the same on every peer:
    // . . . . .   y=2   (3,2) touches the anchor's side, (2,2) only its
    // . . A . .   y=3   corner, so the side comes first.
    let grid = utils::grid(10, 10);

    let scan = vec![utils::nav(3, 3), utils::nav(3, 2), utils::nav(2, 3)];
    for footing in [Footing::Free, Footing::Shared] {
        let found = search::find_placements_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::ONE),
            CellSize::ONE,
            8,
            3,
            footing,
        );

        assert_eq!(found, scan, "on empty ground both footings answer alike");
    }
}

#[test]
fn placements_never_overlap_each_other() {
    // A 2×2 spawn at the anchor (3,3) covers (3,3)..(4,4), so every cell of
    // ring 1 would overlap it — the second placement is the cell of ring 2
    // nearest the anchor that clears it, (2,1), whose own 2×2 covers
    // (2,1)..(3,2).
    let grid = utils::grid(10, 10);

    for footing in [Footing::Free, Footing::Shared] {
        let found = search::find_placements_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(3, 3), CellSize::ONE),
            CellSize::new(2, 2),
            8,
            2,
            footing,
        );

        assert_eq!(
            found,
            vec![utils::nav(3, 3), utils::nav(2, 1)],
            "neither footing sets one down on top of another"
        );
    }
}

#[test]
fn placements_run_out_with_room() {
    // Everything is held but (2,2) and (0,0), and (1,1) is water. What is held
    // is room for a shared footing and not for a free one; what the terrain
    // refuses is room for neither.
    let mut grid = utils::grid(5, 5);
    for y in 0..5 {
        for x in 0..5 {
            grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
        }
    }
    grid.set_occupied(utils::GROUND, utils::nav(2, 2), false);
    grid.set_occupied(utils::GROUND, utils::nav(0, 0), false);
    grid.set_terrain_blocked(utils::GROUND, utils::nav(1, 1));

    let asked = |footing| {
        search::find_placements_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(2, 2), CellSize::ONE),
            CellSize::ONE,
            8,
            4,
            footing,
        )
    };

    assert_eq!(
        asked(Footing::Free),
        vec![utils::nav(2, 2), utils::nav(0, 0)],
        "four asked, two free: what fits is set down, rather than nothing at all"
    );
    assert_eq!(
        asked(Footing::Shared),
        vec![
            utils::nav(2, 2),
            utils::nav(2, 1),
            utils::nav(1, 2),
            utils::nav(3, 2)
        ],
        "a shared footing takes held ground in scan order, and never the water"
    );
}

#[test]
fn placements_ring_entity_standing_on_its_own_cells() {
    // What a cast summons around a standing 3×3 building: the building holds
    // its own footprint, so ring 0 yields nothing and the four stand at the
    // middle of each side of it rather than along one edge.
    // . . . . x . . . .   y=3   x = the placements: (5,3), (3,5), (7,5) and
    // . . . B B B . . .   y=4       (5,7), one to each side
    // . . x B B B x . .   y=5   B = the building at (4,4), 3×3
    // . . . B B B . . .   y=6
    // . . . . x . . . .   y=7
    let mut grid = utils::grid(10, 10);
    for y in 4..7 {
        for x in 4..7 {
            grid.set_occupied(utils::GROUND, utils::nav(x, y), true);
        }
    }

    let asked = |footing| {
        search::find_placements_near(
            &grid,
            utils::GROUND,
            CellRect::new(utils::nav(4, 4), CellSize::new(3, 3)),
            CellSize::ONE,
            8,
            4,
            footing,
        )
    };

    assert_eq!(
        asked(Footing::Free),
        vec![
            utils::nav(5, 3),
            utils::nav(3, 5),
            utils::nav(7, 5),
            utils::nav(5, 7)
        ],
        "a free footing lands on no cell the building holds"
    );
    assert_eq!(
        asked(Footing::Shared),
        vec![
            utils::nav(5, 5),
            utils::nav(5, 4),
            utils::nav(4, 5),
            utils::nav(6, 5)
        ],
        "a shared one lies under it, starting at its middle"
    );
}

#[test]
fn placements_stand_about_middle_rather_than_first_corner() {
    // Two spilled out of a 3×3 hall stand either side of its middle, not in
    // the corner the rows are scanned from: what a wide footprint leaves
    // behind belongs about its middle.
    let grid = utils::grid(10, 10);

    let found = search::find_placements_near(
        &grid,
        utils::GROUND,
        CellRect::new(utils::nav(4, 4), CellSize::new(3, 3)),
        CellSize::ONE,
        8,
        2,
        Footing::Shared,
    );

    assert_eq!(found, vec![utils::nav(5, 5), utils::nav(5, 4)]);
}

#[test]
fn placement_middle_of_even_footprint_lies_between_its_cells() {
    // A 4×4 at (3, 3) has no middle cell: the four around the middle are
    // equally close, so they come first with the tie going to the row and
    // then the column — where a scan from the corner would have answered
    // with the whole of its top row.
    let grid = utils::grid(12, 12);

    let found = search::find_placements_near(
        &grid,
        utils::GROUND,
        CellRect::new(utils::nav(3, 3), CellSize::new(4, 4)),
        CellSize::ONE,
        8,
        4,
        Footing::Shared,
    );

    assert_eq!(
        found,
        vec![
            utils::nav(4, 4),
            utils::nav(5, 4),
            utils::nav(4, 5),
            utils::nav(5, 5)
        ],
        "the four cells about the middle come before the rest of the footprint"
    );
}

#[test]
fn placements_of_none_find_nothing() {
    let grid = utils::grid(5, 5);

    let found = search::find_placements_near(
        &grid,
        utils::GROUND,
        CellRect::new(utils::nav(2, 2), CellSize::ONE),
        CellSize::ONE,
        8,
        0,
        Footing::Free,
    );

    assert!(
        found.is_empty(),
        "a ring of no cells is asked for, so no cell comes back"
    );
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
