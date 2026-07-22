//! Flee response: fleeing-stance entities drop their orders and run from
//! whatever damages them.

mod utils;

use ferrets_pathfinder::{astar, nav_pos::NavPos};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        resource::ResourceSourceComponent,
        stance::{Stance, StanceComponent},
    },
    spawn,
};

#[test]
fn damaged_worker_abandons_harvest_and_runs() {
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
    // The enemy sentry auto-engages the harvesting worker.
    let (_, _) = spawn::spawn_entity(world, "sentry", utils::pos(13, 10), Some(1)).unwrap();

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 60);
    let world = app.world_mut();
    // The worker survives by running: alive, and no longer at the mine.
    assert!(world.get_entity(worker).is_ok(), "worker fled and survived");
    let distance = astar::chebyshev(utils::cell_of(world, worker), utils::cell_of(world, mine));
    assert!(
        distance > 2,
        "worker left the mine, got distance {distance}"
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
    assert_ne!(utils::cell_of(world, runner), NavPos::new(10, 10));
}
