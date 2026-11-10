//! Move order: movers path without claiming cells; an off-map point is ignored.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{command::PlayerCommand, map::Map};

#[test]
fn point_commands_off_map_are_ignored() {
    // The map is 32 cells tall: y 40 lies past its last row. A soldier sent
    // there by Move, AttackMove or Patrol takes no order and stays put.
    let commands = [
        PlayerCommand::Move {
            target: utils::point("10", "40"),
            flush: true,
        },
        PlayerCommand::AttackMove {
            target: utils::point("10", "40"),
            flush: true,
        },
        PlayerCommand::Patrol {
            target: utils::point("10", "40"),
            flush: true,
        },
    ];
    for command in commands {
        let mut app = utils::orders_app();
        let (soldier, soldier_id) =
            utils::create_entity(app.world_mut(), "soldier", utils::cell(10, 10), Some(0)).unwrap();
        utils::select(&mut app, soldier_id);
        let named = format!("{command:?}");
        utils::push_command(&mut app, command);

        utils::run_ticks(&mut app, 10);
        assert!(utils::order_queue_is_empty(app.world(), soldier), "{named}");
        assert_eq!(
            utils::anchor_of(app.world(), soldier),
            CellPos::new(10, 10),
            "{named}"
        );
    }
}

#[test]
fn point_command_to_where_body_stands_is_done_at_once() {
    // A soldier lined up on (10, 10) under the cell model, its middle (10.5,
    // 10.5), is sent there by Move and by AttackMove: each is done the tick
    // it applies, with no walk, and the soldier has not moved.
    let commands = [
        PlayerCommand::Move {
            target: utils::pos(10, 10),
            flush: true,
        },
        PlayerCommand::AttackMove {
            target: utils::pos(10, 10),
            flush: true,
        },
    ];
    for command in commands {
        let mut app = utils::orders_app();
        let (soldier, soldier_id) =
            utils::create_entity(app.world_mut(), "soldier", utils::cell(10, 10), Some(0)).unwrap();
        utils::select(&mut app, soldier_id);
        let named = format!("{command:?}");
        utils::push_command(&mut app, command);

        utils::run_ticks(&mut app, utils::APPLY);
        assert!(utils::order_queue_is_empty(app.world(), soldier), "{named}");
        assert_eq!(
            utils::position_of(app.world(), soldier),
            utils::pos(10, 10),
            "{named}"
        );
    }
}

#[test]
fn passable_entities_never_claim_cells() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (ghost, ghost_id) =
        utils::create_entity(world, "ghost", utils::cell(5, 5), Some(0)).unwrap();

    // The ghost stands at (5,5) without claiming the cell, so a solid soldier
    // can be placed right on top of it.
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_occupied(utils::GROUND, CellPos::new(5, 5))
    );
    let (_, _) = utils::create_entity(world, "soldier", utils::cell(5, 5), Some(0)).unwrap();
    assert!(
        world
            .resource::<Map>()
            .nav_grid()
            .is_occupied(utils::GROUND, CellPos::new(5, 5))
    );

    // The ghost walks away — it collides with the world for pathing, but its
    // crossings leave no occupancy trail.
    utils::select(&mut app, ghost_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::anchor_of(app.world_mut(), ghost), CellPos::new(9, 5));
    assert!(
        !app.world_mut()
            .resource::<Map>()
            .nav_grid()
            .is_occupied(utils::GROUND, CellPos::new(9, 5))
    );
}
