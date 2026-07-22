//! Patrol order: ping-ponging between the start position and the target,
//! engaging hostiles on the way and resuming the route afterward.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{command::PlayerCommand, spawn};

//
// ─── Route ──────────────────────────────────────────────────────────────────
//

#[test]
fn patrol_ping_pongs_between_endpoints() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(5, 10), Some(0)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Patrol {
            target: utils::pos(12, 10),
            flush: true,
        },
    );

    // Two legs take ~28 ticks; 80 covers both endpoints more than once.
    let cells = trace(&mut app, sentry, 80);
    assert!(cells.contains(&NavPos::new(12, 10)), "reaches the target");
    let first_arrival = cells
        .iter()
        .position(|&c| c == NavPos::new(12, 10))
        .unwrap();
    assert!(
        cells[first_arrival..].contains(&NavPos::new(5, 10)),
        "returns to the start"
    );
    // Still patrolling — the order never finishes on its own.
    assert!(!utils::order_queue_is_empty(app.world_mut(), sentry));
}

//
// ─── Engagement ─────────────────────────────────────────────────────────────
//

#[test]
fn patrol_engages_en_route_and_resumes_route() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(5, 10), Some(0)).unwrap();
    let (barracks, _) = spawn::spawn_entity(world, "barracks", utils::pos(8, 12), Some(1)).unwrap();

    utils::select(&mut app, sentry_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Patrol {
            target: utils::pos(12, 10),
            flush: true,
        },
    );

    // Long enough to raze the barracks (~40 ticks of swings) and walk the
    // route afterward.
    let cells = trace(&mut app, sentry, 200);
    utils::assert_despawned(app.world_mut(), barracks);
    let last_quarter = &cells[150..];
    assert!(
        last_quarter.contains(&NavPos::new(12, 10)) && last_quarter.contains(&NavPos::new(5, 10)),
        "route resumed after the fight"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Runs `ticks` one at a time, collecting the cell the entity stands on each tick.
fn trace(app: &mut App, entity: Entity, ticks: u32) -> Vec<NavPos> {
    let mut cells = Vec::new();
    for _ in 0..ticks {
        utils::run_ticks(app, 1);
        cells.push(utils::cell_of(app.world_mut(), entity));
    }
    cells
}
