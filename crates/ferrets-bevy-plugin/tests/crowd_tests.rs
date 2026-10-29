//! Crowd resolution: head-on swaps, idle allies stepping aside, and crowds
//! settling around a shared destination.
//!
//! End states are asserted exactly: the cell model is deterministic
//! lockstep math, so a drifted settle — however plausible — is a desync.
//! Changing a pinned value must be a conscious decision.

mod utils;

use bevy::prelude::App;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{order_queue::OrderQueueComponent, stance::Stance},
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
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
    let (left, left_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    let (right, right_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);

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
fn head_on_movers_of_different_domains_swap_in_corridor() {
    // A sealed corridor, the right-hand mover keeping to dry land: its domain
    // is not the soldier's, but it occupies the same layer with the same
    // footprint, and each lands on a cell of its own route — so they trade
    // places as equals do.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &[], |_, _| {});
    corridor(&mut app);
    let (left, left_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    let (right, right_id) = utils::create_owned(&mut app, "landlubber", 6, 5, 0);

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

    // In a sealed corridor only a swap gets them past each other.
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, left), CellPos::new(7, 5));
    assert_eq!(utils::cell_of(world, right), CellPos::new(2, 5));
    assert!(utils::order_queue_is_empty(world, left));
    assert!(utils::order_queue_is_empty(world, right));
}

#[test]
fn wide_walk_reaches_goal_and_reclaims_footprint() {
    let mut app = utils::orders_app();
    // A cell-model wide walk end to end: claims are law here, so the wagon
    // must carry its whole 2x2 claim along the route and settle it exactly.
    let (wagon, id) = utils::create_owned(&mut app, "wagon", 2, 2, 0);

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
    // cell contract. Nobody teleports through: the idle soldier is asked
    // aside, and the only way aside is ahead of the wagon.
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 0..32 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 7), true);
        }
    }
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 2, 5, 0);

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
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 8, 5, 0);

    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    let wagon_cell = utils::cell_of(world, wagon);
    let soldier_cell = utils::cell_of(world, soldier);
    // The exact end state: the soldier, asked aside down the only way open,
    // walks ahead of the wagon to (14,5), the nearest cell clear of the cells
    // the wagon's walk still enters, and the wagon covers its goal with
    // footprint cells 12..=13. The soldier is still ahead: never swapped.
    assert_eq!(wagon_cell, CellPos::new(12, 5));
    assert_eq!(soldier_cell, CellPos::new(14, 5));
}

#[test]
fn idle_ally_steps_aside() {
    let mut app = utils::orders_app();
    let (mover, mover_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);

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
    let (bystander, _) = utils::create_owned(&mut app, "soldier", 6, 5, 0);

    utils::run_ticks(&mut app, 80);

    assert_eq!(utils::cell_of(app.world_mut(), mover), CellPos::new(8, 5));
    // The yield walks the ally to the nearest cell off the cells the step
    // enters, the row above first: straight up, off the walked line.
    assert_eq!(
        utils::cell_of(app.world_mut(), bystander),
        CellPos::new(6, 4)
    );
}

#[test]
fn ally_standing_its_ground_is_walked_around_not_asked_aside() {
    let mut app = utils::orders_app();
    let (mover, mover_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);

    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let (bystander, bystander_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    utils::select(&mut app, bystander_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );

    utils::run_ticks(&mut app, 80);

    // Idle but standing its ground, the ally holds as an enemy would: the walk
    // goes round it to its goal, and the ally never moves.
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, mover), CellPos::new(8, 5));
    assert_eq!(utils::cell_of(world, bystander), CellPos::new(6, 5));
}

#[test]
fn walk_ends_beside_ally_standing_its_ground_on_its_goal() {
    let mut app = utils::orders_app();
    let (mover, mover_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    let (bystander, bystander_id) = utils::create_owned(&mut app, "soldier", 8, 5, 0);
    utils::select(&mut app, bystander_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );
    // Six cells at two ticks a cell from the order's start on the third tick:
    // (7, 5) on the twelfth. The ally standing its ground on the goal makes
    // no way, but it is a settled ally there, so the walk ends beside it on
    // the thirteenth rather than grinding at its cell.
    utils::run_ticks(&mut app, 12);
    assert!(!utils::order_queue_is_empty(app.world_mut(), mover));
    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, mover));
    assert_eq!(utils::cell_of(world, mover), CellPos::new(7, 5));
    assert_eq!(utils::cell_of(world, bystander), CellPos::new(8, 5));
}

#[test]
fn idle_ally_steps_aside_only_onto_its_terrain() {
    // The meeting of the test above, the row above the bystander water: the
    // mover's domain takes it, but the landlubber's takes land alone, so its
    // step aside cannot go up.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &[(5, 4), (6, 4), (7, 4)], |_, _| {});
    let (mover, mover_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);

    utils::select(&mut app, mover_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let (bystander, _) = utils::create_owned(&mut app, "landlubber", 6, 5, 0);

    utils::run_ticks(&mut app, 80);

    // The yield takes the nearest cell off the walk still on land: the row
    // below, the row above it took in the test above being water.
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, mover), CellPos::new(8, 5));
    assert_eq!(utils::cell_of(world, bystander), CellPos::new(6, 6));
}

#[test]
fn crowd_ordered_to_one_point_settles() {
    let mut app = utils::orders_app();
    let goal = CellPos::new(5, 7);
    let starts = [(2, 2), (4, 2), (6, 2), (8, 2), (2, 4), (8, 4)];

    let soldiers: Vec<_> = starts
        .iter()
        .map(|&(x, y)| utils::create_owned(&mut app, "soldier", x, y, 0))
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
    // The exact settle: the goal taken, the rest ringed beside it — the cells
    // the hierarchy's lanes lead them to and the yields walk them aside to,
    // around (5,7).
    assert_eq!(
        cells,
        vec![
            CellPos::new(4, 6),
            CellPos::new(4, 7),
            CellPos::new(5, 6),
            goal,
            CellPos::new(6, 6),
            CellPos::new(7, 6),
        ]
    );
}

#[test]
fn fanned_group_move_shares_one_corridor() {
    let mut app = utils::orders_app();

    let soldiers = [
        utils::create_owned(&mut app, "soldier", 2, 2, 0),
        utils::create_owned(&mut app, "soldier", 4, 2, 0),
        utils::create_owned(&mut app, "soldier", 3, 4, 0),
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
            CellPos::new(29, 28),
            CellPos::new(29, 29),
        ]
    );
}

#[test]
fn attacker_takes_free_cell_beside_working_ally() {
    let mut app = utils::orders_app();
    let (depot, depot_id) = utils::create_owned(&mut app, "depot", 10, 10, 1);
    let (_, first) = utils::create_owned(&mut app, "soldier", 8, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "soldier", 8, 11, 0);
    for id in [first, second_id] {
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::Attack {
                target: AttackTarget::Entity(depot_id),
                flush: true,
            },
        );
    }

    // Both plan the same reach cell, (9,10); the first takes it and strikes,
    // and the second, finding it held by an ally at work, takes the free cell
    // below it at once instead of waiting on it.
    utils::run_ticks(&mut app, 7);
    assert_eq!(utils::cell_of(app.world_mut(), second), CellPos::new(9, 11));
    // Two blades from the start, each striking every fourth tick and the pair in
    // turn: blows of 10 land on ticks 7, 9, … 25, and the tenth fells the
    // depot's 100.
    utils::run_ticks(&mut app, 18);
    assert_eq!(utils::health(&app, depot), FixedU64::ZERO);
}

#[test]
fn attacker_takes_free_cell_beside_ally_standing_its_ground() {
    let mut app = utils::orders_app();
    let (depot, depot_id) = utils::create_owned(&mut app, "depot", 10, 10, 1);
    let (_, guard_id) = utils::create_owned(&mut app, "worker", 9, 10, 0);
    utils::select(&mut app, guard_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    let (attacker, attacker_id) = utils::create_owned(&mut app, "soldier", 7, 10, 0);
    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(depot_id),
            flush: true,
        },
    );

    // The walk reaches (8, 10) on the fifth tick and finds its reach cell,
    // (9, 10), held by an ally that stands its ground: it takes another reach
    // cell at once, (8, 9) on the seventh tick and (9, 9) on the tenth, and the
    // first blow of 10 lands on the twelfth, not after the waits and detour of
    // a held step.
    utils::run_ticks(&mut app, 11);
    assert_eq!(utils::health(&app, depot), FixedU64::from_num(100));
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::cell_of(app.world_mut(), attacker),
        CellPos::new(9, 9)
    );
    assert_eq!(utils::health(&app, depot), FixedU64::from_num(90));
}

#[test]
fn walk_onto_cell_held_by_enemy_ends_beside_it() {
    let mut app = utils::orders_app();
    utils::create_owned(&mut app, "wagon", 10, 10, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 4, 10, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 10),
            flush: true,
        },
    );

    // At half a cell a tick the walk stands at (9,10) on the twelfth tick, and
    // from the thirteenth the step into the idle enemy wagon's cell is
    // refused. No other cell is the point, so no route leads elsewhere: the
    // walk waits as on any held step, five ticks, and the escalation after
    // them grows its acceptance to one cell, which ends it beside the wagon
    // on the nineteenth. It stays ended until the soldier turns on the wagon
    // on the twenty-third.
    // Still waiting on the fourteenth, where giving the walk up at once would
    // have ended it.
    utils::run_ticks(&mut app, 14);
    assert!(!utils::order_queue_is_empty(app.world_mut(), walker));
    utils::run_ticks(&mut app, 4);
    assert!(!utils::order_queue_is_empty(app.world_mut(), walker));
    utils::run_ticks(&mut app, 1);
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, walker), CellPos::new(9, 10));
    assert!(utils::order_queue_is_empty(world, walker));
}

#[test]
fn walk_waits_on_held_cell_short_of_its_goal() {
    let mut app = utils::orders_app();
    corridor(&mut app);
    utils::create_owned(&mut app, "ox", 6, 5, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 5),
            flush: true,
        },
    );

    // The idle enemy ox holds (6,5), short of the goal at (10,5). The walk
    // reaches (5,5) and works through its wait and detour rungs before giving
    // up on the twenty-first tick — where routing at once, as a held goal
    // does, would end it on the tenth — and stays ended until it turns on the
    // ox on the twenty-third.
    utils::run_ticks(&mut app, 20);
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, walker), CellPos::new(5, 5));
    assert!(!utils::order_queue_is_empty(world, walker));
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), walker));
}

#[test]
fn walk_waits_on_goal_held_by_walking_ally() {
    let mut app = utils::orders_app();
    corridor(&mut app);
    utils::create_owned(&mut app, "ox", 9, 5, 1);
    let (_, ally_id) = utils::create_owned(&mut app, "soldier", 8, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    // The ally on the goal is walking — its move to (12,5) waits behind the
    // ox — so the walker waits on it rather than giving the goal up.
    utils::select(&mut app, ally_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 5),
            flush: true,
        },
    );
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );

    // Still waiting at (7,5) on the fourteenth tick, where giving the goal up
    // at once would have ended the walk; it settles beside the ally on the
    // fifteenth, once the ally's own walk has given up and it stands idle.
    utils::run_ticks(&mut app, 14);
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, walker), CellPos::new(7, 5));
    assert!(!utils::order_queue_is_empty(world, walker));
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), walker));
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Walls rows 4 and 6 across the map, leaving row 5 a one-wide corridor.
fn corridor(app: &mut App) {
    let world = app.world_mut();
    let mut map = world.resource_mut::<Map>();
    for x in 0..32 {
        map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
        map.set_static_occupied(utils::GROUND, CellPos::new(x, 6), true);
    }
}
