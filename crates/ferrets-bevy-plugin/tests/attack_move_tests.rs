//! Attack-move order: moving with en-route engagement, resuming toward the
//! destination after each fight, and degrading to a plain move for the unarmed.

mod utils;

use bevy::prelude::*;
use ferrets_content::pool_def::PoolId;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    command::PlayerCommand, components::stance::Stance, entity_def, map::Map,
};

//
// ─── En-route engagement ────────────────────────────────────────────────────
//

#[test]
fn attack_move_destroys_enemy_near_path_and_reaches_destination() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(2, 10), Some(0)).unwrap();
    let (barracks, _) =
        utils::create_entity(world, "barracks", utils::cell(10, 12), Some(1)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 140);
    let world = app.world_mut();
    utils::assert_despawned(world, barracks);
    assert_eq!(utils::anchor_of(world, sentry), CellPos::new(20, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn attack_move_reengages_until_path_is_clear() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(2, 10), Some(0)).unwrap();
    let (first, _) = utils::create_entity(world, "barracks", utils::cell(7, 12), Some(1)).unwrap();
    let (second, _) = utils::create_entity(world, "barracks", utils::cell(15, 8), Some(1)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 260);
    let world = app.world_mut();
    utils::assert_despawned(world, first);
    utils::assert_despawned(world, second);
    assert_eq!(utils::anchor_of(world, sentry), CellPos::new(20, 10));
}

#[test]
fn hold_fire_unit_still_engages_under_explicit_attack_move() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(2, 10), Some(0)).unwrap();
    let (barracks, _) =
        utils::create_entity(world, "barracks", utils::cell(10, 12), Some(1)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 140);
    utils::assert_despawned(app.world_mut(), barracks);
}

//
// ─── Plain movement ─────────────────────────────────────────────────────────
//

#[test]
fn attack_move_without_enemies_reaches_destination() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(2, 10), Some(0)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 50);
    let world = app.world_mut();
    assert_eq!(utils::anchor_of(world, sentry), CellPos::new(20, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn unarmed_unit_attack_moves_like_plain_move() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        utils::create_entity(world, "worker", utils::cell(2, 10), Some(0)).unwrap();
    let (ghost, _) = utils::create_entity(world, "ghost", utils::cell(10, 12), Some(1)).unwrap();

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 60);
    let world = app.world_mut();
    assert_eq!(utils::anchor_of(world, worker), CellPos::new(20, 10));
    assert_eq!(
        entity_def::pool_value(world, ghost, PoolId::HEALTH).unwrap(),
        20
    );
}

#[test]
fn crowded_attack_move_ends_where_move_does() {
    // Four soldiers in a column, (6, 8)..(6, 11), sent under the continuous
    // model to one point, (10.5, 10.5): one takes it, the rest are pressed
    // around it. Each attack-move ends where its walk ended, as the move
    // does: the same places, every order done.
    let mut ended = Vec::new();
    for attack in [false, true] {
        let mut app = utils::continuous_orders_app();
        let target = utils::point("10.5", "10.5");
        let mut soldiers = Vec::new();
        for y in 8..12 {
            let (soldier, soldier_id) =
                utils::create_entity(app.world_mut(), "soldier", utils::cell(6, y), Some(0))
                    .unwrap();
            utils::select(&mut app, soldier_id);
            let command = if attack {
                PlayerCommand::AttackMove {
                    target,
                    flush: true,
                }
            } else {
                PlayerCommand::Move {
                    target,
                    flush: true,
                }
            };
            utils::push_command(&mut app, command);
            soldiers.push(soldier);
        }

        utils::run_ticks(&mut app, 200);
        let world = app.world();
        for &soldier in &soldiers {
            assert!(
                utils::order_queue_is_empty(world, soldier),
                "attack: {attack}"
            );
        }
        ended.push(
            soldiers
                .iter()
                .map(|&soldier| utils::position_of(world, soldier))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(ended[0], ended[1]);
}

#[test]
fn attack_move_brings_wide_body_where_move_does() {
    // The wagon covers (5..6, 5..6), its middle at (6, 6). Sent to the point
    // (11, 6), a move and an attack-move both bring its middle there: anchored
    // at (11 − 1, 6 − 1) = (10, 5), under either movement model.
    let apps: [fn() -> App; 2] = [utils::orders_app, utils::continuous_orders_app];
    for app in apps {
        for attack in [false, true] {
            let mut app = app();
            let (wagon, wagon_id) =
                utils::create_entity(app.world_mut(), "wagon", utils::cell(5, 5), Some(0)).unwrap();

            let target = utils::point("11", "6");
            utils::select(&mut app, wagon_id);
            let command = if attack {
                PlayerCommand::AttackMove {
                    target,
                    flush: true,
                }
            } else {
                PlayerCommand::Move {
                    target,
                    flush: true,
                }
            };
            utils::push_command(&mut app, command);

            // Five cells at half a cell a tick, and the command's delay.
            utils::run_ticks(&mut app, 30);
            let world = app.world_mut();
            let model = world.resource::<Map>().movement_model();
            let named = format!("{model:?}, attack: {attack}");
            assert_eq!(utils::position_of(world, wagon), target, "{named}");
            assert_eq!(
                utils::anchor_of(world, wagon),
                CellPos::new(10, 5),
                "{named}"
            );
            assert!(utils::order_queue_is_empty(world, wagon), "{named}");
        }
    }
}
