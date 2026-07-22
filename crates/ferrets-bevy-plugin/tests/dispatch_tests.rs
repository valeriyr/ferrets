//! SendToEntity command dispatch: resolving a target into the right order
//! (attack a hostile, follow a target whose resource the carrier cannot carry).

mod utils;

use ferrets_pathfinder::astar;
use ferrets_simulation::{
    command::PlayerCommand, components::resource::ResourceSourceComponent,
    resources::PlayerResources, spawn,
};

#[test]
fn send_to_entity_attacks_hostiles() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (enemy, enemy_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: enemy_id,
            flush: true,
        },
    );

    // The dispatch resolved to an attack: the enemy is chased down and killed.
    utils::run_ticks(&mut app, 18);
    utils::assert_despawned(app.world_mut(), enemy);
}

#[test]
fn send_to_entity_falls_through_to_follow_for_uncarryable_kinds() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    let (tree, tree_id) = spawn::spawn_entity(world, "tree", utils::pos(10, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 10;

    // The worker carries only gold, so a wood source resolves to a follow.
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 11);
    {
        let world = app.world_mut();
        assert!(astar::chebyshev(utils::cell_of(world, worker), utils::cell_of(world, tree)) <= 1);
    }

    // Nothing was harvested: the stockpile and the source are untouched.
    let world = app.world_mut();
    assert_eq!(world.resource::<PlayerResources>().amount(0, "wood"), 0);
    assert_eq!(
        world.get::<ResourceSourceComponent>(tree).unwrap().amount,
        10
    );
}
