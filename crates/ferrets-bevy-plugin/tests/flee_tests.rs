//! Flee response: fleeing-stance entities run from whatever damages them — on
//! their own idle initiative only, never over a player's command.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        health::HealthComponent,
        order_queue::OrderQueueComponent,
        resource::ResourceSourceComponent,
        stance::{Stance, StanceComponent},
    },
    resources::PlayerResources,
    session::GameSession,
    spawn,
};

#[test]
fn commanded_worker_keeps_harvesting_under_fire() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(9, 10), Some(0)).unwrap();
    let (mine, mine_id) = spawn::spawn_entity(world, "mine", utils::pos(10, 10), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 100;
    spawn::spawn_entity(world, "depot", utils::pos(4, 10), Some(0)).unwrap();
    // An enemy to be hit by, parked far outside its own acquisition range, so
    // the only damage is the hit staged below.
    let (_, sentry_id) = spawn::spawn_entity(world, "sentry", utils::pos(26, 10), Some(1)).unwrap();

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
    let mut health = world.get_mut::<HealthComponent>(worker).unwrap();
    health.apply_damage(FixedU64::from_num(5));
    health.record_hit(sentry_id, tick);
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
        spawn::spawn_entity(world, "sentry", utils::pos(10, 10), Some(0)).unwrap();
    let (enemy, _) = spawn::spawn_entity(world, "sentry", utils::pos(13, 10), Some(1)).unwrap();

    utils::select(&mut app, runner_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::Flee,
        },
    );

    utils::run_ticks(&mut app, 60);
    let world = app.world_mut();
    // Both live — two defending sentries this close would have fought to a
    // death by now, so survival on both sides is the fleeing itself.
    assert!(world.get_entity(runner).is_ok(), "runner survived");
    assert!(world.get_entity(enemy).is_ok(), "enemy was never engaged");
    assert_eq!(
        world.get::<StanceComponent>(runner).unwrap().0,
        Stance::Flee
    );
    // And it ran: the runner is no longer on its starting cell.
    assert_ne!(utils::cell_of(world, runner), CellPos::new(10, 10));
}
