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
            map.nav_grid_mut()
                .set_occupied_by(utils::GROUND, CellPos::new(x, 4), true);
            map.nav_grid_mut()
                .set_occupied_by(utils::GROUND, CellPos::new(x, 6), true);
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
