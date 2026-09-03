//! Attack-move order: moving with en-route engagement, resuming toward the
//! destination after each fight, and degrading to a plain move for the unarmed.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{health::HealthComponent, stance::Stance},
    spawn,
};

//
// ─── En-route engagement ────────────────────────────────────────────────────
//

#[test]
fn attack_move_destroys_enemy_near_path_and_reaches_destination() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::create_entity(world, "sentry", utils::pos(2, 10), Some(0)).unwrap();
    let (barracks, _) =
        spawn::create_entity(world, "barracks", utils::pos(10, 12), Some(1)).unwrap();

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
    assert_eq!(utils::cell_of(world, sentry), CellPos::new(20, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn attack_move_reengages_until_path_is_clear() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::create_entity(world, "sentry", utils::pos(2, 10), Some(0)).unwrap();
    let (first, _) = spawn::create_entity(world, "barracks", utils::pos(7, 12), Some(1)).unwrap();
    let (second, _) = spawn::create_entity(world, "barracks", utils::pos(15, 8), Some(1)).unwrap();

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
    assert_eq!(utils::cell_of(world, sentry), CellPos::new(20, 10));
}

#[test]
fn hold_fire_unit_still_engages_under_explicit_attack_move() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, sentry_id) = spawn::create_entity(world, "sentry", utils::pos(2, 10), Some(0)).unwrap();
    let (barracks, _) =
        spawn::create_entity(world, "barracks", utils::pos(10, 12), Some(1)).unwrap();

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
        spawn::create_entity(world, "sentry", utils::pos(2, 10), Some(0)).unwrap();

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
    assert_eq!(utils::cell_of(world, sentry), CellPos::new(20, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn unarmed_unit_attack_moves_like_plain_move() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::create_entity(world, "worker", utils::pos(2, 10), Some(0)).unwrap();
    let (ghost, _) = spawn::create_entity(world, "ghost", utils::pos(10, 12), Some(1)).unwrap();

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
    assert_eq!(utils::cell_of(world, worker), CellPos::new(20, 10));
    assert_eq!(world.get::<HealthComponent>(ghost).unwrap().current(), 20);
}
