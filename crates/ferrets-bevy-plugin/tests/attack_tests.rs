//! Attack order: landing hits, chasing out-of-range targets, and stopping.

mod utils;

use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        attack::AttackComponent,
        buffs::{BuffDef, StackRule},
        dying::DyingComponent,
        health::HealthComponent,
        stats::{Modifier, ModifierOp, StatId},
    },
    content::{entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry},
    entity_index::EntityIndex,
    game_loop,
    map::Map,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
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

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );

    // The first hit lands at the damage point.
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

    utils::select(&mut app, attacker_id);
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

    utils::select(&mut app, attacker_id);
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
fn send_to_entity_does_not_attack_ally() {
    // Players 0 and 1 share team 1. A right-click (SendToEntity) from player 0's
    // soldier onto its adjacent ally resolves to Follow, not Attack, so the ally
    // takes no damage. (An explicit Attack command would still be honored — that
    // is force-fire, a separate path.)
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 1, 4, 2),
        );
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();

    let world = app.world_mut();
    let (_, actor_id) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (ally, ally_id) = spawn::spawn_entity(world, "soldier", utils::pos(6, 5), Some(1)).unwrap();

    utils::select(&mut app, actor_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: ally_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 6);
    // The ally kept full health — the right-click never became an attack.
    assert_eq!(
        app.world_mut()
            .get::<HealthComponent>(ally)
            .unwrap()
            .current(),
        30,
    );
}

#[test]
fn attack_order_with_missing_target_finishes() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();

    utils::select(&mut app, attacker_id);
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

#[test]
fn shortened_attack_cycle_still_lands_hits() {
    // The soldier's authored cycle is 4 ticks with the hit landing on tick 2. A
    // debuff shortens the cycle to 1, leaving the authored hit beyond its end —
    // the phase counter then runs 1..=1 and would never reach 2, so without the
    // clamp this attacker would never deal damage at all.
    let mut app = utils::combat_app();
    let (attacker, attacker_id) =
        spawn::spawn_entity(app.world_mut(), "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(app.world_mut(), "dummy", utils::pos(6, 5), None).unwrap();

    let hasty = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_buff(
            "hasty",
            BuffDef {
                modifiers: vec![Modifier {
                    stat: StatId::ATTACK_PERIOD,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(-3),
                }],
                duration: None,
                stack_rule: StackRule::Refresh,
            },
        );
    game_loop::stats::apply_buff(app.world_mut(), attacker, hasty);

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 4);

    // With the cycle clamped to 1 a hit lands every tick, so two land in the two
    // ticks after the command arrives — exactly lethal for the 20-hp dummy. Under
    // the authored 4-tick cycle only one hit could have landed by now.
    let health = app.world().get::<HealthComponent>(target).unwrap();
    assert!(
        health.is_dead(),
        "expected the shortened cycle to land two hits, target at {} hp",
        health.displayed()
    );
}

#[test]
fn attack_gives_up_on_walled_in_target() {
    // The target sits inside a ring of solid dummies, so no path reaches a cell
    // within the soldier's range of 1. The chase must notice it made no progress
    // and finish the order rather than walk forever.
    let mut app = utils::combat_app();
    let (_, attacker_id) =
        spawn::spawn_entity(app.world_mut(), "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(app.world_mut(), "dummy", utils::pos(15, 15), None).unwrap();
    for (x, y) in [
        (14, 14),
        (15, 14),
        (16, 14),
        (14, 15),
        (16, 15),
        (14, 16),
        (15, 16),
        (16, 16),
    ] {
        spawn::spawn_entity(app.world_mut(), "dummy", utils::pos(x, y), None)
            .expect("wall segment must be placeable");
    }

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: target_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 120);

    let attacker = utils::single_owned_of_type(app.world_mut(), "soldier", 0);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), attacker),
        "the chase must give up on an unreachable target"
    );
    assert_eq!(
        app.world()
            .get::<HealthComponent>(target)
            .unwrap()
            .current(),
        FixedU64::from_num(20),
        "the walled-in target must take no damage"
    );
}
