//! Turrets as parts: what several guns on one body do about several targets, what
//! an order does to all of them at once, and what a body's own weapon does
//! alongside them.
//!
//! North is the way `y` decreases, so a target placed above a body is due north
//! of it.

mod utils;

use bevy::prelude::{App, Entity};
use ferrets_math::{facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{stance::Stance, turret::TurretsComponent},
    entity_index::EntityIndex,
    impacts::PendingImpacts,
    order::AttackTarget,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── Several guns on one body ───────────────────────────────────────────────
//

/// Guns told to spread pass over what another on the same body already holds, so
/// a keep facing four raiders answers four of them rather than shooting one four
/// times.
#[test]
fn spreading_keep_answers_every_attacker() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, _) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(0)).unwrap();
    let raiders: Vec<SimulationId> = [(9, 8), (16, 9), (9, 16), (16, 16)]
        .into_iter()
        .map(|(x, y)| {
            spawn::spawn_entity(world, "hulk", utils::pos(x, y), Some(1))
                .unwrap()
                .1
        })
        .collect();

    utils::run_ticks(&mut app, 12);

    let mut worked = quarries_of(&app, keep);
    worked.sort();
    worked.dedup();
    assert_eq!(worked.len(), 4, "four guns, four fights");
    for raider in raiders {
        assert!(
            worked.contains(&raider),
            "nothing answered the raider {raider:?}"
        );
    }
}

/// Guns told to focus each take the target they would take alone, so they agree.
#[test]
fn focused_keep_puts_every_gun_on_one_attacker() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, _) =
        spawn::spawn_entity(world, "focused_keep", utils::pos(10, 10), Some(0)).unwrap();
    for (x, y) in [(9, 8), (16, 9), (9, 16), (16, 16)] {
        spawn::spawn_entity(world, "hulk", utils::pos(x, y), Some(1)).unwrap();
    }

    utils::run_ticks(&mut app, 12);

    let mut worked = quarries_of(&app, keep);
    worked.sort();
    worked.dedup();
    assert_eq!(worked.len(), 1, "one fight, four guns on it");
}

/// Spreading is not decided once: a second attacker arriving later takes guns off
/// the first, because a keep with four guns and two sides answers both.
#[test]
fn spreading_keep_answers_attacker_that_arrives_later() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, _) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(0)).unwrap();
    let (_, first) = spawn::spawn_entity(world, "hulk", utils::pos(9, 8), Some(1)).unwrap();
    utils::run_ticks(&mut app, 12);
    assert_eq!(
        quarries_of(&app, keep),
        vec![first; 4],
        "everything it had was on the only attacker there was"
    );

    // The other side, well after the guns settled on the first.
    let (_, second) =
        spawn::spawn_entity(app.world_mut(), "hulk", utils::pos(16, 16), Some(1)).unwrap();
    utils::run_ticks(&mut app, 8);

    let worked = quarries_of(&app, keep);
    assert_eq!(
        worked.iter().filter(|&&id| id == first).count(),
        2,
        "half the guns held the first attacker"
    );
    assert_eq!(
        worked.iter().filter(|&&id| id == second).count(),
        2,
        "and half came round on the second, {worked:?}"
    );
}

/// A spreading body still puts everything it has on a lone attacker: passing over
/// what a sibling holds is what it does while there is anything else to take.
#[test]
fn spreading_keep_falls_back_onto_lone_attacker() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, _) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(0)).unwrap();
    let (_, raider) = spawn::spawn_entity(world, "hulk", utils::pos(9, 8), Some(1)).unwrap();

    utils::run_ticks(&mut app, 12);

    assert_eq!(
        quarries_of(&app, keep),
        vec![raider; 4],
        "every gun took the only thing there was"
    );
}

/// The fresh-attacker preference is a call to arms for a gun with nothing to do:
/// a gun already fighting does not drop a nearer fight for whoever hit last, or
/// its swing would be dragged between attackers at every scan and land on none.
#[test]
fn working_gun_holds_nearer_fight_against_fresh_attacker() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, keep_id) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(1)).unwrap();
    let (_, near) = spawn::spawn_entity(world, "hulk", utils::pos(9, 9), Some(0)).unwrap();
    utils::run_ticks(&mut app, 12);
    assert_eq!(
        quarries_of(&app, keep),
        vec![near; 4],
        "every gun was on the one attacker there was"
    );

    // A second attacker set on the far corner — the freshest mark there is, and
    // farther from the first gun's corner than what that gun holds.
    let (_, soldier_id) =
        spawn::spawn_entity(app.world_mut(), "soldier", utils::pos(15, 15), Some(0)).unwrap();
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(keep_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        quarries_of(&app, keep)[0],
        near,
        "the first gun kept the nearer fight it already had"
    );
}

//
// ─── What an order does to them ─────────────────────────────────────────────
//

/// An attack still queued behind other business has not named a fight: the guns
/// answer what is beside the road until the body takes the order up.
#[test]
fn queued_attack_does_not_bind_guns_early() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 5), Some(0)).unwrap();
    let (_, near) = spawn::spawn_entity(world, "hulk", utils::pos(6, 6), Some(1)).unwrap();
    let (_, far) = spawn::spawn_entity(world, "hulk", utils::pos(12, 9), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(15, 5),
            flush: true,
        },
    );
    // Queued behind the walk, inside the gun's own notice the whole way.
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(far),
            flush: false,
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(
        quarries_of(&app, wagon),
        vec![near],
        "the gun works what it rolls past, not the fight still waiting its turn"
    );
}

/// What an ordered attack closes to is the longest reach among the weapons that
/// can serve the target: a long gun for the air is no reason to stop short of
/// walking a short spear onto what crawls.
#[test]
fn attack_closes_to_reach_of_weapon_serving_target() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, escort_id) = spawn::spawn_entity(world, "escort", utils::pos(2, 10), Some(0)).unwrap();
    // Ten cells out: exactly the anti-air gun's reach, five times the spear's —
    // judged by the gun, the escort would already be standing where it can never
    // land a hit.
    let (_, dummy_id) = spawn::spawn_entity(world, "dummy", utils::pos(12, 10), Some(1)).unwrap();

    utils::select(&mut app, escort_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(dummy_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 40);

    assert_eq!(
        app.world().resource::<EntityIndex>().alive(dummy_id),
        None,
        "the escort walked its spear into reach and killed what it was sent at"
    );
}

/// A body that fights only from turrets has no look in a fight: ordered onto
/// something its gun cannot yet reach, the walls stand square while the gun
/// comes round.
#[test]
fn ordered_keep_does_not_turn_its_walls() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, keep_id) = spawn::spawn_entity(world, "bastion", utils::pos(5, 5), Some(0)).unwrap();
    // Beyond the gun's reach of eight from the keep's edge, inside its notice of
    // twelve: the waiting-to-reach path is exactly where a hull was turned.
    let (_, target_id) = spawn::spawn_entity(world, "hulk", utils::pos(16, 5), Some(1)).unwrap();

    utils::select(&mut app, keep_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(
        utils::facing_of(app.world(), keep),
        Facing::SOUTH,
        "the walls stood square"
    );
    assert_ne!(
        utils::bearing_of(app.world(), keep),
        Facing::SOUTH,
        "while the gun came round on what it waits for"
    );
}

/// A weapon that follows bodies is never pointed at bare ground: ordered onto a
/// cell, the body's own spear holds while the gun that throws at places works it.
#[test]
fn ordered_cell_is_worked_by_cell_aimed_gun_alone() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, bombardier_id) =
        spawn::spawn_entity(world, "bombardier", utils::pos(5, 10), Some(0)).unwrap();
    let (bystander, _) = spawn::spawn_entity(world, "hulk", utils::pos(8, 10), Some(1)).unwrap();

    utils::select(&mut app, bombardier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Position(utils::pos(8, 10)),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);

    assert_eq!(
        utils::health(&app, bystander),
        490,
        "one landed lob, and no spear thrust at bare ground"
    );
}

/// The mirror: an ordered cell leaves a gun that follows bodies free — and with
/// nothing hostile about, free means working nothing.
#[test]
fn ordered_cell_leaves_instant_gun_free() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (battery, battery_id) =
        spawn::spawn_entity(world, "battery", utils::pos(5, 10), Some(0)).unwrap();

    utils::select(&mut app, battery_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Position(utils::pos(8, 10)),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);

    assert_eq!(
        app.world()
            .get::<TurretsComponent>(battery)
            .expect("a body with guns carries their state")
            .0[0]
            .quarry,
        None,
        "a bare cell binds nothing that cannot be sent to one"
    );
}

/// An attack-move stops for what any weapon notices, asked of the weapon that
/// names the stat: a body with no acquisition range of its own still answers
/// what its gun would.
#[test]
fn attack_move_stops_for_what_turret_notices() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, prowler_id) =
        spawn::spawn_entity(world, "prowler", utils::pos(2, 10), Some(0)).unwrap();
    let (_, dummy_id) = spawn::spawn_entity(world, "dummy", utils::pos(10, 7), Some(1)).unwrap();

    utils::select(&mut app, prowler_id);
    utils::push_command(
        &mut app,
        PlayerCommand::AttackMove {
            target: utils::pos(18, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 60);

    assert_eq!(
        app.world().resource::<EntityIndex>().alive(dummy_id),
        None,
        "the walk stopped for what the gun noticed, and the gun killed it"
    );
}

//
// ─── What a stance does to them ──────────────────────────────────────────────
//

/// A body told to hold its fire holds every gun on it: a fight a gun had picked
/// for itself is given up, not merely left unrenewed.
#[test]
fn held_fire_takes_guns_off_what_they_hold() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, keep_id) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "hulk", utils::pos(9, 8), Some(1)).unwrap();

    utils::run_ticks(&mut app, 12);
    assert_eq!(
        quarries_of(&app, keep).len(),
        4,
        "the guns had picked their fight"
    );

    utils::select(&mut app, keep_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(
        quarries_of(&app, keep).is_empty(),
        "and holding fire took every gun off it"
    );
}

//
// ─── A gun's cycle ───────────────────────────────────────────────────────────
//

/// A gun's cycle outlives the numbers it was counted against — a morph or a
/// debuff can leave the phase beyond a shortened cycle — and it wraps there
/// instead of counting past the end forever.
#[test]
fn gun_recovers_from_phase_beyond_its_cycle() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, keep_id) = spawn::spawn_entity(world, "bastion", utils::pos(5, 5), Some(0)).unwrap();
    // Due south, the way the gun is mounted, so its narrow arc gates nothing.
    let (target, target_id) =
        spawn::spawn_entity(world, "hulk", utils::pos(5, 10), Some(1)).unwrap();

    utils::select(&mut app, keep_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);
    let before = utils::health(&app, target);
    assert!(before < 500, "the fight is under way");

    // A phase far beyond the four-tick cycle, as a shortened cycle leaves one.
    app.world_mut()
        .get_mut::<TurretsComponent>(keep)
        .expect("a body with guns carries their state")
        .0[0]
        .phase = 50;
    utils::run_ticks(&mut app, 6);

    assert!(
        utils::health(&app, target) < before,
        "the cycle wrapped and the gun fought on"
    );
}

/// An order is given to the body, so every gun that can take what it named drops
/// what it found for itself and works that instead.
#[test]
fn order_binds_every_gun_that_can_take_it() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (keep, keep_id) =
        spawn::spawn_entity(world, "spreading_keep", utils::pos(10, 10), Some(0)).unwrap();
    for (x, y) in [(9, 8), (16, 9), (9, 16)] {
        spawn::spawn_entity(world, "hulk", utils::pos(x, y), Some(1)).unwrap();
    }
    let (_, named) = spawn::spawn_entity(world, "hulk", utils::pos(16, 16), Some(1)).unwrap();

    // Spread had them on four different raiders; the order overrides all of it.
    utils::run_ticks(&mut app, 12);
    utils::select(&mut app, keep_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(named),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 4);

    assert_eq!(
        quarries_of(&app, keep),
        vec![named; 4],
        "the body was told what to kill, so every gun on it is killing that"
    );
}

/// A body that fights only from turrets still takes an Attack order: it walks
/// itself into range and holds while its guns do the shooting.
#[test]
fn turret_only_body_walks_to_what_it_was_ordered_onto() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(17, 10), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);

    assert_eq!(
        utils::health(&app, target),
        0,
        "it killed what it was sent at"
    );
    assert!(
        utils::position_of(app.world(), wagon).x > utils::pos(10, 10).x,
        "having walked most of the way there to do it"
    );
}

//
// ─── A body weapon beside a turret ──────────────────────────────────────────
//

/// Two guns on one body are two weapons, not one: the body's own swings while the
/// turret does, and the target takes both.
#[test]
fn body_weapon_and_turret_both_fight() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    spawn::spawn_entity(world, "gunship", utils::pos(5, 10), Some(0)).unwrap();
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(5, 8), Some(1)).unwrap();

    // Ten ticks is one cycle apiece: two weapons, two hits, and a dummy carrying
    // twenty health has none left.
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::health(&app, target), 0, "both guns landed a hit");
}

/// A gun answers what its body's own weapon cannot: they reach different layers,
/// and each fights what it can.
#[test]
fn turret_answers_what_body_weapon_cannot_reach() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    spawn::spawn_entity(world, "flak_post", utils::pos(5, 5), Some(0)).unwrap();
    let (kite, _) = spawn::spawn_entity(world, "kite", utils::pos(5, 3), Some(1)).unwrap();

    utils::run_ticks(&mut app, 16);

    assert_eq!(
        utils::health(&app, kite),
        10,
        "the gun worked what flew over, whatever the body could not touch"
    );
}

/// An order arrives at the body's longest reach, which may be a turret's: the
/// body's own weapon still fires only inside its own, and waits out the rest.
#[test]
fn body_weapon_waits_beyond_its_own_reach() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, longarm_id) =
        spawn::spawn_entity(world, "longarm", utils::pos(5, 10), Some(0)).unwrap();
    // Five cells out: inside the gun's eight, far beyond the spear's two.
    let (target, target_id) =
        spawn::spawn_entity(world, "hulk", utils::pos(10, 10), Some(1)).unwrap();

    utils::select(&mut app, longarm_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);

    assert_eq!(
        utils::health(&app, target),
        490,
        "one hit from the gun, and none from a spear four times out of its reach"
    );
}

/// A shot leaves the gun that fired it, not the middle of the body carrying it: a
/// keep with a gun on one corner throws from that corner.
#[test]
fn shot_leaves_from_gun_that_fired_it() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    // The gun sits three cells in from the keep's own origin and spans two, so it
    // stands at (14, 14) while the keep's middle is (12.5, 12.5).
    spawn::spawn_entity(world, "shell_keep", utils::pos(10, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "hulk", utils::pos(17, 17), Some(1)).unwrap();

    // Ten ticks in, its first shell is in the air.
    utils::run_ticks(&mut app, 10);

    let shots: Vec<FixedUVec2> = app
        .world()
        .resource::<PendingImpacts>()
        .in_flight()
        .iter()
        .map(|shot| shot.origin)
        .collect();
    assert_eq!(
        shots,
        vec![utils::pos(14, 14)],
        "the shell left the corner the gun is mounted on"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// What each of `entity`'s guns is working, in mounted order.
fn quarries_of(app: &App, entity: Entity) -> Vec<SimulationId> {
    app.world()
        .get::<TurretsComponent>(entity)
        .expect("a body with guns carries their state")
        .0
        .iter()
        .filter_map(|turret| match turret.quarry {
            Some(AttackTarget::Entity(target)) => Some(target),
            Some(AttackTarget::Position(_)) | None => None,
        })
        .collect()
}
