//! Guard order: escorting the ward, engaging what threatens it, and finishing
//! when the ward is gone.

mod utils;

use ferrets_pathfinder::{astar, nav_pos::NavPos};
use ferrets_simulation::{command::PlayerCommand, spawn};

//
// ─── Escorting ──────────────────────────────────────────────────────────────
//

#[test]
fn guard_follows_moving_ward() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(5, 10), Some(0)).unwrap();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(6, 10), Some(0)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: worker_id,
            flush: true,
        },
    );
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 18),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 80);
    let world = app.world_mut();
    let distance = astar::chebyshev(utils::cell_of(world, sentry), utils::cell_of(world, worker));
    assert!(
        distance <= 3,
        "guard trails the ward, got distance {distance}"
    );
    // Still guarding.
    assert!(!utils::order_queue_is_empty(world, sentry));
}

#[test]
fn guard_of_building_holds_station() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(3, 10), Some(0)).unwrap();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: barracks_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 40);
    let world = app.world_mut();
    let distance = astar::chebyshev(
        utils::cell_of(world, sentry),
        utils::cell_of(world, barracks),
    );
    assert!(
        distance <= 3,
        "guard stations near the ward, got {distance}"
    );
    assert!(!utils::order_queue_is_empty(world, sentry));
}

#[test]
fn guard_finishes_when_ward_dies() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(5, 10), Some(0)).unwrap();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(6, 10), Some(0)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: worker_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 4);

    spawn::destroy_entity(app.world_mut(), worker);
    utils::run_ticks(&mut app, 6);

    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, sentry));
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(5, 10));
}

//
// ─── Engagement ─────────────────────────────────────────────────────────────
//

#[test]
fn guard_engages_attacker_threatening_ward() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, guard_id) = spawn::spawn_entity(world, "sentry", utils::pos(8, 10), Some(0)).unwrap();
    let (_, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    // The enemy sentry will engage the barracks on its own; the guard must
    // answer for it.
    let (enemy, _) = spawn::spawn_entity(world, "sentry", utils::pos(15, 10), Some(1)).unwrap();

    utils::select(&mut app, guard_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: barracks_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 100);
    utils::assert_despawned(app.world_mut(), enemy);
}

//
// ─── Refusal ────────────────────────────────────────────────────────────────
//

#[test]
fn guard_on_hostile_ward_is_refused() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(5, 10), Some(0)).unwrap();
    // Well outside the sentry's acquire range, so only a guard order (escorting
    // toward it) could ever move the sentry — isolating the refusal.
    let (_, enemy_id) = spawn::spawn_entity(world, "worker", utils::pos(25, 10), Some(1)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Guard {
            target: enemy_id,
            flush: true,
        },
    );

    // A hostile ward would just become the guard's own scan target — refused
    // outright, leaving the sentry idle where it stands.
    utils::run_ticks(&mut app, 5);
    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, sentry));
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(5, 10));
}
