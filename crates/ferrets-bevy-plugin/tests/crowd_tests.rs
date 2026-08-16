//! Crowd resolution: head-on swaps, idle allies stepping aside, and crowds
//! settling around a shared destination.
//!
//! End states are asserted exactly: the cell model is deterministic
//! lockstep math, so a drifted settle — however plausible — is a desync.
//! Changing a pinned value must be a conscious decision.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::order_queue::OrderQueueComponent,
    map::Map,
};

#[test]
fn head_on_movers_swap_in_corridor() {
    let mut app = utils::orders_app();
    // A one-cell corridor along y=5: head-on movers cannot route around, so
    // only the swap resolves the meeting.
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 1..=8 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 6), true);
        }
    }
    let (left, left_id) = utils::spawn_owned(&mut app, "soldier", 2, 5, 0);
    let (right, right_id) = utils::spawn_owned(&mut app, "soldier", 6, 5, 0);

    utils::select(&mut app, left_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(7, 5),
            flush: true,
        },
    );
    utils::select(&mut app, right_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(2, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 120);

    assert_eq!(utils::cell_of(app.world_mut(), left), CellPos::new(7, 5));
    assert_eq!(utils::cell_of(app.world_mut(), right), CellPos::new(2, 5));
}

#[test]
fn wide_walk_reaches_goal_and_reclaims_footprint() {
    let mut app = utils::orders_app();
    // A cell-model wide walk end to end: claims are law here, so the wagon
    // must carry its whole 2x2 claim along the route and settle it exactly.
    let (wagon, id) = utils::spawn_owned(&mut app, "wagon", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 6),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 200);

    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, wagon), CellPos::new(10, 6));
    let grid = world.resource::<Map>().nav_grid();
    for (x, y) in [(10, 6), (11, 6), (10, 7), (11, 7)] {
        assert!(
            grid.is_claimed_by(utils::GROUND, CellPos::new(x, y)),
            "the settled footprint cell ({x}, {y}) is unclaimed"
        );
    }
    for (x, y) in [(2, 2), (3, 2), (2, 3), (3, 3)] {
        assert!(
            !grid.is_claimed_by(utils::GROUND, CellPos::new(x, y)),
            "the spawn footprint cell ({x}, {y}) was never released"
        );
    }
}

#[test]
fn head_on_unequal_sizes_never_swap() {
    let mut app = utils::orders_app();
    // A corridor exactly the wagon's height: the soldier cannot slip past,
    // and the swap rung is refused between unequal sizes — trading claims
    // with a body of another footprint would corrupt the one-claimant-per-
    // cell contract. Nobody teleports through; the meeting ends with both
    // giving up on their own side.
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 0..32 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 7), true);
        }
    }
    let (wagon, wagon_id) = utils::spawn_owned(&mut app, "wagon", 2, 5, 0);

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 5),
            flush: true,
        },
    );
    // Let the walk plan its clear straight line, then park a soldier on it:
    // the meeting arises mid-walk, where the crowd ladder runs. An equal
    // body would resolve it by the swap rung; the unequal pair must not.
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let (soldier, _) = utils::spawn_owned(&mut app, "soldier", 8, 5, 0);

    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    let wagon_cell = utils::cell_of(world, wagon);
    let soldier_cell = utils::cell_of(world, soldier);
    // The exact end state: the wagon walks up to the soldier — footprint
    // cells 6..=7, abutting it — burns its escalations against a blocker it
    // may neither swap with nor push planning through, and gives up in
    // place. The soldier never moves: a corridor its own width leaves the
    // yield rung nowhere to step either.
    assert_eq!(wagon_cell, CellPos::new(6, 5));
    assert_eq!(soldier_cell, CellPos::new(8, 5));
}

#[test]
fn idle_ally_steps_aside() {
    let mut app = utils::orders_app();
    let (mover, mover_id) = utils::spawn_owned(&mut app, "soldier", 2, 5, 0);

    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );
    // Let the walk plan its straight line, then park an ally on it.
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let (bystander, _) = utils::spawn_owned(&mut app, "soldier", 6, 5, 0);

    utils::run_ticks(&mut app, 80);

    assert_eq!(utils::cell_of(app.world_mut(), mover), CellPos::new(8, 5));
    // The yield steps the ally off the walked line, diagonally forward —
    // the aside cell farthest from the mover's direction of travel.
    assert_eq!(
        utils::cell_of(app.world_mut(), bystander),
        CellPos::new(7, 4)
    );
}

#[test]
fn crowd_ordered_to_one_point_settles() {
    let mut app = utils::orders_app();
    let goal = CellPos::new(5, 7);
    let starts = [(2, 2), (4, 2), (6, 2), (8, 2), (2, 4), (8, 4)];

    let soldiers: Vec<_> = starts
        .iter()
        .map(|&(x, y)| utils::spawn_owned(&mut app, "soldier", x, y, 0))
        .collect();
    for (_, id) in &soldiers {
        utils::select(&mut app, *id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(goal.x, goal.y),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 400);

    let mut cells: Vec<CellPos> = Vec::new();
    for (soldier, _) in &soldiers {
        let world = app.world_mut();
        assert!(
            world
                .entity(*soldier)
                .get::<OrderQueueComponent>()
                .is_some_and(|queue| queue.0.is_empty()),
            "every walk must settle instead of grinding"
        );
        cells.push(utils::cell_of(world, *soldier));
    }
    cells.sort_unstable();
    // The exact settle: the goal taken, the rest ringed beside it.
    assert_eq!(
        cells,
        vec![
            CellPos::new(3, 7),
            CellPos::new(4, 6),
            CellPos::new(5, 6),
            goal,
            CellPos::new(6, 6),
            CellPos::new(6, 7),
        ]
    );
}

#[test]
fn fanned_group_move_shares_one_corridor() {
    let mut app = utils::orders_app();
    // Rebuild the map with a hierarchy for the ground mask, so the group
    // plans hierarchically (the plain harness map carries no abstractions).
    utils::install_map(
        &mut app,
        ferrets_geometry::projection::Projection::Isometric,
        ferrets_simulation::movement_model::MovementModel::Cell,
    );

    let soldiers = [
        utils::spawn_owned(&mut app, "soldier", 2, 2, 0),
        utils::spawn_owned(&mut app, "soldier", 4, 2, 0),
        utils::spawn_owned(&mut app, "soldier", 3, 4, 0),
    ];
    for (index, (_, id)) in soldiers.iter().enumerate() {
        utils::push_command(
            &mut app,
            PlayerCommand::SelectById {
                id: *id,
                mode: if index == 0 {
                    SelectMode::Replace
                } else {
                    SelectMode::Add
                },
            },
        );
    }
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(29, 29),
            flush: true,
        },
    );

    // The tick the fanned orders first process plans exactly one corridor
    // for the whole group — all three start in one cluster.
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(
        utils::selection(&app).len(),
        3,
        "the whole group must be selected"
    );

    utils::run_ticks(&mut app, 400);
    let mut cells: Vec<CellPos> = soldiers
        .iter()
        .map(|(soldier, _)| utils::cell_of(app.world_mut(), *soldier))
        .collect();
    cells.sort_unstable();
    // The exact settle: the goal taken, the followers packed beside it.
    assert_eq!(
        cells,
        vec![
            CellPos::new(28, 28),
            CellPos::new(28, 29),
            CellPos::new(29, 29),
        ]
    );
}
