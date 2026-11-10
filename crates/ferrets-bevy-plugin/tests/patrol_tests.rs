//! Patrol order: ping-ponging between the start position and the target,
//! engaging hostiles on the way and resuming the route afterward.

mod utils;

use bevy::prelude::*;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::command::PlayerCommand;

//
// ─── Route ──────────────────────────────────────────────────────────────────
//

#[test]
fn patrol_ping_pongs_between_endpoints() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(5, 10), Some(0)).unwrap();

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
    assert!(cells.contains(&CellPos::new(12, 10)), "reaches the target");
    let first_arrival = cells
        .iter()
        .position(|&c| c == CellPos::new(12, 10))
        .unwrap();
    assert!(
        cells[first_arrival..].contains(&CellPos::new(5, 10)),
        "returns to the start"
    );
    // Still patrolling — the order never finishes on its own.
    assert!(!utils::order_queue_is_empty(app.world_mut(), sentry));
}

#[test]
fn wide_patroller_keeps_its_endpoints() {
    // The wagon covers (5..6, 10..11), its middle at (6, 11), and patrols to
    // the point (11, 11): one end anchored at (5, 10), the other at
    // (11 − 1, 11 − 1) = (10, 10), round after round, under either movement
    // model.
    let apps: [fn() -> App; 2] = [utils::orders_app, utils::continuous_orders_app];
    for app in apps {
        let mut app = app();
        let (wagon, wagon_id) =
            utils::create_entity(app.world_mut(), "wagon", utils::cell(5, 10), Some(0)).unwrap();

        utils::select(&mut app, wagon_id);
        utils::push_command(
            &mut app,
            PlayerCommand::Patrol {
                target: utils::point("11", "11"),
                flush: true,
            },
        );

        // Five cells a leg at half a cell a tick: 120 ticks walk several
        // rounds.
        let cells = trace(&mut app, wagon, 120);
        let east = cells.iter().map(|cell| cell.x).max().unwrap();
        let west = cells[30..].iter().map(|cell| cell.x).min().unwrap();
        assert_eq!((west, east), (5, 10));
        assert!(cells.iter().all(|cell| cell.y == 10));
    }
}

//
// ─── Engagement ─────────────────────────────────────────────────────────────
//

#[test]
fn patrol_engages_en_route_and_resumes_route() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(5, 10), Some(0)).unwrap();
    let (barracks, _) =
        utils::create_entity(world, "barracks", utils::cell(8, 12), Some(1)).unwrap();

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
        last_quarter.contains(&CellPos::new(12, 10)) && last_quarter.contains(&CellPos::new(5, 10)),
        "route resumed after the fight"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Runs `ticks` one at a time, collecting the cell the entity stands on each tick.
fn trace(app: &mut App, entity: Entity, ticks: u32) -> Vec<CellPos> {
    let mut cells = Vec::new();
    for _ in 0..ticks {
        utils::run_ticks(app, 1);
        cells.push(utils::anchor_of(app.world_mut(), entity));
    }
    cells
}
