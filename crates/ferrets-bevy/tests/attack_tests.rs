//! Attack order: landing hits, chasing out-of-range targets, and stopping.

mod utils;

use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{attack::AttackComponent, dying::DyingComponent, health::HealthComponent},
    entity_index::EntityIndex,
    map::Map,
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn attack_kills_adjacent_target() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(6, 5), None).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: attacker_id });
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );

    // The first hit lands after the aiming phase.
    utils::run_ticks(&mut app, 4);
    assert!(
        app.world_mut()
            .get::<HealthComponent>(target)
            .is_some_and(|h| h.current() < 20)
    );

    // The target reaches 0 hp and starts dying: out of the alive set, but it
    // still holds its cell until the dying phase completes.
    utils::run_ticks(&mut app, 4);
    assert!(app.world_mut().get::<DyingComponent>(target).is_some());
    {
        let world = app.world_mut();
        assert_eq!(world.resource::<EntityIndex>().alive(target_id), None);
        assert!(
            world
                .resource::<Map>()
                .nav_grid()
                .is_occupied_by(utils::GROUND, NavPos::new(6, 5))
        );
    }

    // The dying phase completes and the entity leaves the world.
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), target);

    // The attack order finishes once the target is gone.
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), attacker));
    assert!(app.world_mut().get::<AttackComponent>(attacker).is_none());
}

#[test]
fn attack_chases_target_out_of_range() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(10, 5), None).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: attacker_id });
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );

    // The attacker walks into range, kills the target, and the corpse is removed.
    utils::run_ticks(&mut app, 21);
    utils::assert_despawned(app.world_mut(), target);

    // The attacker stopped within attack range of the target's cell.
    assert_eq!(utils::cell_of(app.world_mut(), attacker), NavPos::new(9, 5));

    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), attacker));
}

#[test]
fn stop_cancels_attack() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(6, 5), None).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: attacker_id });
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );

    // Wait for the first hit, then order a stop.
    utils::run_ticks(&mut app, 4);
    assert!(
        app.world_mut()
            .get::<HealthComponent>(target)
            .is_some_and(|h| h.current() < 20)
    );
    utils::push_command(&mut app, PlayerCommand::Stop);

    utils::run_ticks(&mut app, 3);
    assert!(utils::order_queue_is_empty(app.world_mut(), attacker));
    let world = app.world_mut();
    assert!(world.get::<AttackComponent>(attacker).is_none());

    // The target survives with partial health.
    let health = world.get::<HealthComponent>(target).unwrap();
    assert!(health.current() > 0);
    assert!(world.get::<DyingComponent>(target).is_none());
}

#[test]
fn attack_order_with_missing_target_finishes() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: attacker_id });
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: SimulationId(999),
            flush: true,
        },
    );

    // The command dispatches on the 3rd tick (2-tick input latency); the order
    // is created then and finishes the same tick because the target is gone.
    utils::run_ticks(&mut app, 3);
    assert!(utils::order_queue_is_empty(app.world_mut(), attacker));
    assert!(app.world_mut().get::<AttackComponent>(attacker).is_none());
}
