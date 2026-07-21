//! Stance-driven idle defense: defend units engage and return home,
//! stand-ground units never move, hold-fire units never engage, and ordered
//! units are never hijacked.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        health::HealthComponent,
        stance::{Stance, StanceComponent},
    },
    session::GameSession,
    spawn,
};

//
// ─── Stance defaults ────────────────────────────────────────────────────────
//

#[test]
fn armed_unit_defaults_to_defend_and_worker_to_flee() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (soldier, _) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (worker, _) = spawn::spawn_entity(world, "worker", utils::pos(7, 5), Some(0)).unwrap();
    let (barracks, _) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();

    assert_eq!(
        world.get::<StanceComponent>(soldier).unwrap().0,
        Stance::Defend
    );
    assert_eq!(
        world.get::<StanceComponent>(worker).unwrap().0,
        Stance::Flee
    );
    // An unarmed immobile building has no initiative to configure.
    assert!(world.get::<StanceComponent>(barracks).is_none());
}

#[test]
fn set_stance_applies_to_owned_selection_only() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (own, own_id) = spawn::spawn_entity(world, "sentry", utils::pos(5, 5), Some(0)).unwrap();
    let (foreign, foreign_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(20, 20), Some(1)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: own_id });
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::run_ticks(&mut app, 2);
    utils::push_command(&mut app, PlayerCommand::SelectById { id: foreign_id });
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::run_ticks(&mut app, 2);

    let world = app.world_mut();
    assert_eq!(
        world.get::<StanceComponent>(own).unwrap().0,
        Stance::HoldFire
    );
    // A foreign unit can be selected but not commanded.
    assert_eq!(
        world.get::<StanceComponent>(foreign).unwrap().0,
        Stance::Defend
    );
}

//
// ─── Idle engagement ────────────────────────────────────────────────────────
//

#[test]
fn defend_unit_destroys_enemy_in_acquire_range_and_returns_home() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, _) = spawn::spawn_entity(world, "sentry", utils::pos(10, 10), Some(0)).unwrap();
    let (barracks, _) =
        spawn::spawn_entity(world, "barracks", utils::pos(13, 10), Some(1)).unwrap();

    // 100 hp at 10 damage per 4-tick swing, plus walking there and back.
    utils::run_ticks(&mut app, 80);

    let world = app.world_mut();
    utils::assert_despawned(world, barracks);
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(10, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn stand_ground_unit_fires_in_weapon_range_and_never_moves() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(10, 10), Some(0)).unwrap();
    let (ghost, _) = spawn::spawn_entity(world, "ghost", utils::pos(14, 10), Some(1)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: sentry_id });
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );

    // The ghost sits within acquire range but outside weapon range: a
    // stand-ground sentry must not walk over.
    utils::run_ticks(&mut app, 30);
    {
        let world = app.world_mut();
        assert_eq!(utils::cell_of(world, sentry), NavPos::new(10, 10));
        assert_eq!(world.get::<HealthComponent>(ghost).unwrap().current(), 20);
    }

    // Adjacent, the ghost is fair game — and the hit sends it fleeing, which
    // the zero leash refuses to follow.
    let (ghost2, _) = {
        let world = app.world_mut();
        spawn::spawn_entity(world, "ghost", utils::pos(11, 10), Some(1)).unwrap()
    };
    utils::run_ticks(&mut app, 30);

    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(10, 10));
    assert!(world.get::<HealthComponent>(ghost2).unwrap().current() < 20);
    assert_ne!(utils::cell_of(world, ghost2), NavPos::new(11, 10));
}

#[test]
fn hold_fire_unit_never_engages() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(10, 10), Some(0)).unwrap();
    let (ghost, _) = spawn::spawn_entity(world, "ghost", utils::pos(11, 10), Some(1)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: sentry_id });
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );

    utils::run_ticks(&mut app, 30);
    let world = app.world_mut();
    assert_eq!(world.get::<HealthComponent>(ghost).unwrap().current(), 20);
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(10, 10));
    assert!(utils::order_queue_is_empty(world, sentry));
}

#[test]
fn ordered_unit_is_not_hijacked_by_idle_engagement() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, sentry_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(2, 10), Some(0)).unwrap();
    let (ghost, _) = spawn::spawn_entity(world, "ghost", utils::pos(10, 12), Some(1)).unwrap();

    // A plain move straight past an enemy: the order is executed as given.
    utils::push_command(&mut app, PlayerCommand::SelectById { id: sentry_id });
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 10),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 50);
    let world = app.world_mut();
    assert_eq!(utils::cell_of(world, sentry), NavPos::new(20, 10));
    assert_eq!(world.get::<HealthComponent>(ghost).unwrap().current(), 20);
}

#[test]
fn leashed_chase_abandons_fled_target_and_returns_home() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (archer, _) = spawn::spawn_entity(world, "archer", utils::pos(10, 10), Some(0)).unwrap();
    let (ghost, _) = spawn::spawn_entity(world, "ghost", utils::pos(13, 10), Some(1)).unwrap();

    // The archer engages at range, the hit sends the ghost fleeing beyond the
    // leash, and the archer gives up the chase and walks home.
    utils::run_ticks(&mut app, 80);

    let world = app.world_mut();
    assert!(world.get::<HealthComponent>(ghost).is_some(), "ghost lives");
    assert!(world.get::<HealthComponent>(ghost).unwrap().current() < 20);
    assert_eq!(utils::cell_of(world, archer), NavPos::new(10, 10));
    assert!(utils::order_queue_is_empty(world, archer));
}

//
// ─── Hit memory ─────────────────────────────────────────────────────────────
//

#[test]
fn fresh_attacker_is_preferred_over_nearer_target() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (sentry, _) = spawn::spawn_entity(world, "sentry", utils::pos(10, 10), Some(0)).unwrap();
    // A passive enemy farther out, and a nearer enemy worker.
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(14, 10), Some(1)).unwrap();
    hold_fire(world, attacker);
    let (nearer, _) = spawn::spawn_entity(world, "worker", utils::pos(11, 12), Some(1)).unwrap();

    // A hit landing this tick makes the far attacker the fresh target.
    let tick = current_tick(world);
    world
        .get_mut::<HealthComponent>(sentry)
        .unwrap()
        .record_hit(attacker_id, tick);

    // The sentry pursues its attacker, leaving the nearer worker untouched —
    // asserted before the memory window lapses.
    utils::run_ticks(&mut app, 20);
    let world = app.world_mut();
    assert!(
        world.get::<HealthComponent>(attacker).unwrap().current() < 30,
        "the attacker was engaged"
    );
    assert_eq!(
        world.get::<HealthComponent>(nearer).unwrap().current(),
        20,
        "the nearer worker was untouched"
    );
}

#[test]
fn stale_attacker_is_not_preferred_over_nearer_target() {
    let mut app = utils::orders_app();
    let (sentry, _) =
        spawn::spawn_entity(app.world_mut(), "sentry", utils::pos(10, 10), Some(0)).unwrap();

    // Carry the clock well past the memory window with no enemies present, so
    // a hit stamped at tick 0 is already stale by the time either enemy exists.
    utils::run_ticks(&mut app, 45);

    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "sentry", utils::pos(14, 10), Some(1)).unwrap();
    hold_fire(world, attacker);
    let (nearer, _) = spawn::spawn_entity(world, "worker", utils::pos(11, 12), Some(1)).unwrap();
    world
        .get_mut::<HealthComponent>(sentry)
        .unwrap()
        .record_hit(attacker_id, 0);

    // The sentry engages the nearer worker, and never chases the stale attacker.
    utils::run_ticks(&mut app, 15);
    let world = app.world_mut();
    assert!(
        world.get::<HealthComponent>(nearer).unwrap().current() < 20,
        "the nearer worker was engaged"
    );
    assert_eq!(
        world.get::<HealthComponent>(attacker).unwrap().current(),
        30,
        "the stale attacker was untouched"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// The session's current tick.
fn current_tick(world: &World) -> u32 {
    world.resource::<GameSession>().tick()
}

/// Sets an entity's stance to hold fire, so it neither engages nor retaliates.
fn hold_fire(world: &mut World, entity: Entity) {
    world.get_mut::<StanceComponent>(entity).unwrap().0 = Stance::HoldFire;
}
