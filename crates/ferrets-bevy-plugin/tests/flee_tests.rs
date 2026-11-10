//! Flee response: fleeing-stance entities run from whatever damages them — on
//! their own idle initiative only, never over a player's command.

mod utils;

use bevy::prelude::*;
use ferrets_content::pool_def::PoolId;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        last_hit,
        order_queue::OrderQueueComponent,
        pools,
        resource::ResourceSourceComponent,
        stance::{Stance, StanceComponent},
    },
    order::Order,
    resources::PlayerResources,
    session::GameSession,
};

#[test]
fn commanded_worker_keeps_harvesting_under_fire() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        utils::create_entity(world, "worker", utils::cell(9, 10), Some(0)).unwrap();
    let (mine, mine_id) = utils::create_entity(world, "mine", utils::cell(10, 10), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 100;
    utils::create_entity(world, "depot", utils::cell(4, 10), Some(0)).unwrap();
    // An enemy to be hit by, parked far outside its own acquisition range, so
    // the only damage is the hit staged below.
    let (_, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(26, 10), Some(1)).unwrap();

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    // One hit, stamped exactly as the flee response reads it, landing after
    // the first delivery banked.
    let world = app.world_mut();
    let tick = world.resource::<GameSession>().tick();
    assert_eq!(world.resource::<PlayerResources>().amount(0, "gold"), 5);
    pools::drain(world, worker, PoolId::HEALTH, FixedU64::from_num(5));
    last_hit::record(world, worker, sentry_id, tick);
    utils::run_ticks(&mut app, 60);

    // Commanded outranks scared: the order stays queued and the harvest keeps
    // producing — a fled worker would have banked nothing more.
    let world = app.world_mut();
    assert!(
        !world
            .get::<OrderQueueComponent>(worker)
            .unwrap()
            .0
            .is_empty(),
        "the harvest order was dropped for the flee response"
    );
    assert_eq!(
        world.resource::<PlayerResources>().amount(0, "gold"),
        25,
        "the deliveries stopped, so the worker fled its job"
    );
}

#[test]
fn fleeing_soldier_runs_instead_of_fighting_back() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (runner, runner_id) =
        utils::create_entity(world, "sentry", utils::cell(10, 10), Some(0)).unwrap();
    let (enemy, _) = utils::create_entity(world, "sentry", utils::cell(13, 10), Some(1)).unwrap();

    utils::select(&mut app, runner_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::Flee,
        },
    );

    // The enemy closes in and lands one blow: 30 − 10 leaves the runner 20.
    utils::run_ticks(&mut app, 25);
    assert_eq!(utils::health(&app, runner), FixedU64::from_num(20));
    // Never struck back: the enemy keeps all 30 of its health.
    assert_eq!(utils::health(&app, enemy), FixedU64::from_num(30));
    let world = app.world_mut();
    assert_eq!(
        world.get::<StanceComponent>(runner).unwrap().0,
        Stance::Flee
    );
    // And it ran: from (10,10) to (4,16), six cells down and to the left, the
    // enemy coming from the right at (13,10).
    assert_eq!(utils::anchor_of(world, runner), CellPos::new(4, 16));
}

#[test]
fn runner_shot_from_inside_holder_flees_from_holder() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 10, 7, 0);
    // In the garrison's reach, idle, on a side the bunker's owner fights, and
    // unarmed, so it flees by default; one row above the bunker's own, one
    // below the row the rifleman boards from.
    let (runner, _) = utils::create_owned(&mut app, "grunt", 13, 9, 2);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    // The shot comes from the rifleman, which is wherever the bunker is, at
    // (10, 10): six cells away from it on each axis, east and north, is
    // (19, 3). From the cell the rifleman boarded by, (10, 8), the flight
    // would go south instead, to (19, 15).
    let mut target = None;
    for _ in 0..40 {
        utils::run_ticks(&mut app, 1);
        let front = app
            .world()
            .get::<OrderQueueComponent>(runner)
            .and_then(|queue| queue.front().map(|entry| entry.order.clone()));
        if let Some(Order::Move { target: at, .. }) = front {
            target = Some(at);
            break;
        }
    }
    assert_eq!(target, Some(utils::pos(19, 3)));
}

#[test]
fn wide_body_flees_onto_its_cells() {
    // Under the continuous model the wagon, unarmed and so fleeing, covers
    // (10..11, 10..11). Hit from the sentry anchored at (4, 10), it runs six
    // cells away on each axis, to stand anchored at (16, 16): its middle at
    // (16 + 1, 16 + 1) = (17, 17).
    let mut app = utils::continuous_orders_app();
    let world = app.world_mut();
    let (wagon, _) = utils::create_entity(world, "wagon", utils::cell(10, 10), Some(0)).unwrap();
    let (_, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(4, 10), Some(1)).unwrap();
    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    let tick = world.resource::<GameSession>().tick();
    last_hit::record(world, wagon, sentry_id, tick);
    assert_eq!(flee_target(&mut app, wagon), Some(utils::point("17", "17")));
}

#[test]
fn wide_body_flees_onto_map_at_far_edge() {
    // The wagon, anchored at (28, 10) on the 32-wide map and hit from the
    // sentry at (20, 10), runs six cells away on each axis: x to
    // min(28 + 6, 31) = 31, y to 16. A 2×2 at (31, 16) would cover column 32,
    // so it stands held on the map, anchored at (32 − 2, 16) = (30, 16): its
    // middle at (30 + 1, 16 + 1) = (31, 17).
    let mut app = utils::continuous_orders_app();
    let world = app.world_mut();
    let (wagon, _) = utils::create_entity(world, "wagon", utils::cell(28, 10), Some(0)).unwrap();
    let (_, sentry_id) =
        utils::create_entity(world, "sentry", utils::cell(20, 10), Some(1)).unwrap();
    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    let tick = world.resource::<GameSession>().tick();
    last_hit::record(world, wagon, sentry_id, tick);
    assert_eq!(flee_target(&mut app, wagon), Some(utils::point("31", "17")));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Runs up to three ticks one at a time, returning the target of the first
/// move `entity` queues.
fn flee_target(app: &mut App, entity: Entity) -> Option<FixedUVec2> {
    for _ in 0..3 {
        utils::run_ticks(app, 1);
        let front = app
            .world()
            .get::<OrderQueueComponent>(entity)
            .and_then(|queue| queue.front().map(|entry| entry.order.clone()));
        if let Some(Order::Move { target, .. }) = front {
            return Some(target);
        }
    }
    None
}
