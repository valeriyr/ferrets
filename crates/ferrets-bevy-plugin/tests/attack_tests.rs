//! Attack order: landing hits, chasing out-of-range targets, and stopping.

mod utils;

use bevy::prelude::{App, Entity};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};

use ferrets_content::{
    attack::{AttackDef, Delivery, Weapon},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
    stats::ModifierOp,
};
use ferrets_math::{
    FixedU64,
    facing::{self, Facing},
    fixed_uvec2::FixedUVec2,
};
use ferrets_physics::body;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        attack::AttackComponent, dying::DyingComponent, health::HealthComponent,
        location::LocationComponent, movement::MoveComponent, stance::Stance,
        turret::TurretsComponent,
    },
    entity_index::EntityIndex,
    game_loop,
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
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
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );

    // The first hit lands at the damage point: ten damage into twenty health.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::health(&app, target), 10);

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
                .is_occupied_by(utils::GROUND, CellPos::new(6, 5))
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
fn wide_attacker_stops_at_its_edge_reach() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    // Range is rect to rect: the ballista's reach is measured from its
    // nearest footprint edge, so it stops a full body earlier than a
    // single-cell attacker with the same range stat would.
    let (ballista, ballista_id) =
        spawn::spawn_entity(world, "ballista", utils::pos(2, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(10, 5), None).unwrap();

    utils::select(&mut app, ballista_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 60);
    utils::assert_despawned(app.world_mut(), target);

    // Anchored at (7,5) the footprint's east edge sits at x=8, two cells
    // from the target — exactly the weapon's range.
    assert_eq!(
        utils::cell_of(app.world_mut(), ballista),
        CellPos::new(7, 5)
    );
}

#[test]
fn wide_attacker_stops_at_its_edge_reach_continuous() {
    // The same edge-reach contract under the continuous model, whose arrival
    // check is a separate code path.
    let mut app = utils::combat_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let world = app.world_mut();
    let (ballista, ballista_id) =
        spawn::spawn_entity(world, "ballista", utils::pos(2, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(10, 5), None).unwrap();

    utils::select(&mut app, ballista_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 80);
    utils::assert_despawned(app.world_mut(), target);

    // A continuous body rests at a fractional position; the cell it stands
    // on for every judgement is its body anchor.
    let position = utils::position_of(app.world_mut(), ballista);
    assert_eq!(body::anchor(position), CellPos::new(7, 5));
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
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );

    // The attacker walks into range, kills the target, and the corpse is removed.
    utils::run_ticks(&mut app, 21);
    utils::assert_despawned(app.world_mut(), target);

    // The attacker stopped within attack range of the target's cell.
    assert_eq!(
        utils::cell_of(app.world_mut(), attacker),
        CellPos::new(9, 5)
    );

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
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );

    // Wait for the first hit, then order a stop.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::health(&app, target), 10);
    utils::push_command(&mut app, PlayerCommand::Stop);

    utils::run_ticks(&mut app, 3);
    assert!(utils::order_queue_is_empty(app.world_mut(), attacker));
    let world = app.world_mut();
    assert!(world.get::<AttackComponent>(attacker).is_none());

    // The target survives on the one hit it took: the stop landed before the
    // second could.
    assert_eq!(utils::health(&app, target), 10);
    assert!(app.world_mut().get::<DyingComponent>(target).is_none());
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
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_attack(
                    AttackDef::new(Weapon::new(utils::GROUND, Delivery::Instant, None)),
                    10,
                    1,
                    1,
                    4,
                    2,
                ),
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
            target: AttackTarget::Entity(SimulationId(999)),
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

    let hasty = utils::register_entity_buff(
        &mut app,
        "hasty",
        EntityStatId::ATTACK_PERIOD,
        ModifierOp::FlatAdd,
        "-3",
        None,
    );
    game_loop::stats::apply_entity_buff(app.world_mut(), attacker, hasty);

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
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
            target: AttackTarget::Entity(target_id),
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

//
// ─── Bringing a gun to bear ─────────────────────────────────────────────────
//

/// A gun on a turret holds its fire until it bears on its target. The keep it
/// sits on never turns: what comes round is the weapon.
#[test]
fn turret_holds_fire_until_it_bears_on_target() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    // Due north of the gun, which starts pointing south as anything freshly put
    // down does — half a circle to come round, at three degrees a tick.
    let (gun, gun_id) = spawn::spawn_entity(world, "bastion", utils::pos(5, 8), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(5, 3), None).unwrap();
    let square = app
        .world()
        .get::<LocationComponent>(gun)
        .expect("the gun has a location")
        .facing;

    utils::select(&mut app, gun_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );

    // Long past the damage point, and still nothing lands: the arc reaches thirty
    // degrees either side of the bearing, which is fifty ticks of coming round
    // away.
    utils::run_ticks(&mut app, 30);
    assert_eq!(
        utils::health(&app, target),
        20,
        "the shot must wait until the gun bears on it"
    );
    assert_eq!(
        app.world().get::<LocationComponent>(gun).unwrap().facing,
        square,
        "and the keep itself must not turn"
    );

    utils::run_ticks(&mut app, 30);
    assert_eq!(
        app.world().resource::<EntityIndex>().alive(target_id),
        None,
        "once it bears it fires, and thirty damage into twenty health kills"
    );
}

/// A gun that cannot close brings itself to bear while its target is still out of
/// reach, and waits: it neither fires nor gives the fight up, because a walk it
/// could never take is not what its patience is for.
#[test]
fn turret_tracks_target_beyond_reach_without_walking() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    // Ten cells north: past the gun's reach of eight, inside the twelve it notices.
    let (gun, gun_id) = spawn::spawn_entity(world, "bastion", utils::pos(5, 15), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(5, 5), None).unwrap();
    let mounted = utils::bearing_of(app.world(), gun);

    utils::select(&mut app, gun_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 60);

    assert_eq!(
        utils::health(&app, target),
        20,
        "out of reach, so nothing lands"
    );
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), gun),
        "and the fight is held rather than given up"
    );
    let bearing = utils::bearing_of(app.world(), gun);
    assert_ne!(bearing, mounted, "meanwhile the gun comes round");
    assert!(
        bearing.distance(Facing::NORTH) < mounted.distance(Facing::NORTH),
        "toward the target it is waiting for, {bearing:?}"
    );
    assert!(
        app.world().get::<MoveComponent>(gun).is_none(),
        "and it never tried to walk"
    );
}

/// A gun on wheels keeps its hull's heading while its gun comes round: the body
/// points where it drove, the turret where it is trained, and aiming never turns
/// the hull. Two values, and the simulation must not let one write the other.
#[test]
fn walking_gun_keeps_hull_heading_while_it_aims() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "gun_wagon", utils::pos(5, 5), Some(0)).unwrap();
    let (_, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(9, 1), None).unwrap();

    // Drive east, which is what gives the hull its heading.
    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 40);
    assert_eq!(
        utils::facing_of(app.world(), wagon),
        Facing::EAST,
        "the walk is what wrote the hull's heading"
    );

    // Then engage what stands due north of where it stopped.
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::NORTH,
        "the gun came round onto its target"
    );
    assert_eq!(
        utils::facing_of(app.world(), wagon),
        Facing::EAST,
        "and the hull kept the heading it drove in on"
    );
}

/// A gun keeps the bearing it was left at when its fight ends. The attack state
/// goes when the order does, so a bearing kept there would snap back to the body's
/// look the moment the last target died — and a keep that never turns would appear
/// to forget where it was pointing.
#[test]
fn gun_keeps_its_bearing_after_its_fight_ends() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (gun, gun_id) = spawn::spawn_entity(world, "bastion", utils::pos(5, 8), Some(0)).unwrap();
    let (_, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(5, 3), None).unwrap();
    let mounted = utils::bearing_of(app.world(), gun);

    utils::select(&mut app, gun_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    // Long enough to come round, fire, and finish: the target dies to one hit.
    utils::run_ticks(&mut app, 120);
    assert_eq!(
        app.world().resource::<EntityIndex>().alive(target_id),
        None,
        "the target is dead, so the fight is over"
    );

    let held = utils::bearing_of(app.world(), gun);
    assert_ne!(
        held, mounted,
        "the gun came round to its target and must not have snapped back"
    );
    // Not all the way onto it: the arc reaches thirty degrees either side, so the
    // gun fires — and stops — as soon as the target is inside it.
    let half_arc = facing::units_of_degrees(FixedU64::from_num(30));
    assert!(
        held.distance(Facing::NORTH) <= half_arc,
        "it stopped where it could fire from, {held:?}"
    );

    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::bearing_of(app.world(), gun),
        held,
        "and holds that bearing while it has nothing to shoot"
    );
}

/// A body-mounted weapon has no turret to come round, so it turns itself — which
/// is what infantry does and what keeps a shot leaving on the tick it is ordered.
#[test]
fn body_mounted_weapon_turns_itself_and_fires_at_once() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) = spawn::spawn_entity(world, "dummy", utils::pos(6, 5), None).unwrap();

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 4);

    assert_eq!(
        app.world()
            .get::<LocationComponent>(attacker)
            .unwrap()
            .facing,
        Facing::EAST,
        "the body comes round to its target itself"
    );
    assert_eq!(
        utils::health(&app, target),
        10,
        "and the hit lands on the damage point, with no arc to wait for"
    );
}

//
// ─── Firing on the move ─────────────────────────────────────────────────────
//

/// A gun authored to fight on the move works what its body rolls past, and the
/// body never stops for it: the order it was given is the only thing steering the
/// wheels.
#[test]
fn rolling_gun_shoots_what_it_drives_past() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    // Three cells north of the road: inside the weapon's reach of four, and never
    // on the way, so nothing about the walk is about the target.
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(11, 7), Some(1)).unwrap();

    // A plain walk east along the road. Nothing here orders a fight.
    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 14);

    assert_eq!(
        utils::health(&app, target),
        10,
        "the gun worked it while the wheels rolled"
    );
    assert_eq!(
        utils::position_of(app.world(), wagon),
        utils::pos(11, 10),
        "six cells further along the road, having never stopped for it"
    );

    utils::run_ticks(&mut app, 6);

    assert_eq!(
        utils::health(&app, target),
        0,
        "the second hit of the cycle"
    );
    assert_eq!(
        utils::position_of(app.world(), wagon),
        utils::pos(14, 10),
        "and still rolling"
    );
}

/// What it notices further off than it shoots, it keeps its gun on and holds its
/// fire at — the same split an emplacement fights by, so the gun is already round
/// when its target comes into reach.
#[test]
fn rolling_gun_tracks_what_it_cannot_yet_reach() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    // Six cells north of the road: inside the acquisition range of eight, outside
    // the weapon's reach of four for the whole drive.
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(11, 4), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    // Fourteen ticks puts the wagon exactly abreast of the target, so the bearing
    // it should be holding is exactly north.
    utils::run_ticks(&mut app, 14);

    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::NORTH,
        "the gun stayed on what it had noticed"
    );
    assert_eq!(
        utils::health(&app, target),
        20,
        "and held its fire, never being in reach of it",
    );
}

/// The arc gates a rolling gun as it gates a standing one: a target well inside
/// reach is not fired at until the gun has come round onto it.
#[test]
fn rolling_gun_holds_fire_until_it_bears_on_target() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(11, 7), Some(1)).unwrap();
    // Left pointing down the road it is not driving: the gun has to come round
    // most of a half turn before the target is inside its sixty-degree arc, and
    // it comes into reach long before it comes to bear.
    world.get_mut::<TurretsComponent>(wagon).unwrap().0[0].bearing = Facing::SOUTH;

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 13);

    assert_eq!(
        utils::health(&app, target),
        20,
        "a gun still coming round has not fired",
    );

    utils::run_ticks(&mut app, 1);

    assert_eq!(
        utils::health(&app, target),
        10,
        "and the hit lands on the tick its swing was started for",
    );
}

/// Two looks, and the fight writes only one of them: the hull holds the heading
/// it drives on while the gun works a target off to the side.
#[test]
fn hull_keeps_its_heading_while_gun_fights() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "dummy", utils::pos(11, 7), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 14);

    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::NORTH,
        "the gun is on the target it is abreast of"
    );
    assert_eq!(
        utils::facing_of(app.world(), wagon),
        Facing::EAST,
        "and the hull is still on the road"
    );
}

//
// ─── Who owns the gun ───────────────────────────────────────────────────────
//

/// An Attack order hands its target to a gun that fights on the move: the gun is
/// trained on it and swinging while the body is still walking, instead of arriving
/// cold and starting there.
#[test]
fn ordered_gun_works_target_while_it_closes() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    // Eight cells east: twice the weapon's reach, so the order must walk for it.
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(13, 10), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(
        utils::position_of(app.world(), wagon),
        FixedUVec2::new(FixedU64::lit("8.5"), FixedU64::from_num(10)),
        "still walking, half a cell short of where it will stop"
    );
    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::EAST,
        "the gun came round on the way rather than on arrival"
    );
    assert_eq!(
        swing_of(&app, wagon),
        1,
        "and the swing is under way before the walk has ended"
    );
    assert_eq!(
        utils::health(&app, target),
        20,
        "the hit has not landed yet"
    );
}

/// A handed-over target is not revisited: the gun works what the order named and
/// leaves what it passes alone, however reachable.
#[test]
fn ordered_gun_keeps_to_target_it_was_given() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    // Eight cells east: what the order named, and inside the range this gun engages
    // at, so it is the gun's fight for the whole drive.
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(13, 10), Some(1)).unwrap();
    // And one it drives right past, two cells off the road and well inside reach.
    let (bystander, _) = spawn::spawn_entity(world, "dummy", utils::pos(9, 8), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::health(&app, target),
        0,
        "the target it was given is dead"
    );
    assert_eq!(
        utils::health(&app, bystander),
        20,
        "and what it drove past was never fired at"
    );
}

/// A stance that picks no targets of its own still fights the one it was ordered
/// onto: what the player named is not the gun's choice to decline.
#[test]
fn held_fire_works_target_it_was_ordered_onto() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(5, 8), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(utils::health(&app, target), 10, "the ordered hit landed");
}

/// A shot at bare ground has no body to hand over, so an order aimed at a place
/// keeps the weapon for itself — and taking it back stops the gun working whatever
/// it last held.
#[test]
fn ground_attack_takes_gun_back_from_its_last_target() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    // A shell thrown at where it aims rather than at what it aims at: the one kind
    // of weapon an order may point at a place.
    let (_, mortar_id) =
        spawn::spawn_entity(world, "rolling_mortar", utils::pos(5, 10), Some(0)).unwrap();
    let (held, held_id) = spawn::spawn_entity(world, "dummy", utils::pos(5, 8), Some(1)).unwrap();
    let (shelled, _) = spawn::spawn_entity(world, "dummy", utils::pos(9, 10), Some(1)).unwrap();

    utils::select(&mut app, mortar_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(held_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 6);
    assert_eq!(
        utils::health(&app, held),
        10,
        "the handed-over fight landed a hit"
    );

    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Position(utils::pos(9, 10)),
            flush: true,
        },
    );
    // Long enough for the new order to reach the queue, be taken up, and land one
    // shell of its own.
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        utils::health(&app, held),
        10,
        "the gun let go of what it held when the order named a place instead"
    );
    assert_eq!(
        utils::health(&app, shelled),
        10,
        "and the order works the place itself, as it always did"
    );
}

/// The same gun on the same wheels, authored to halt for its fights, rolls past
/// the same target without firing — which is what every other armed type in the
/// game does, and the only thing the conduct changes.
#[test]
fn halting_gun_drives_past_without_shooting() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "gun_wagon", utils::pos(5, 10), Some(0)).unwrap();
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(11, 7), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::health(&app, target),
        20,
        "a weapon that halts to fight does not fight while walking",
    );
}

/// The same gun fires where it stands, which is what proves the drive-past above
/// held its fire because of the walk rather than because it could not fire at all.
#[test]
fn halting_gun_fires_where_it_stands() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "gun_wagon", utils::pos(5, 10), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(8, 10), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    // Two ticks of command latency, a hit three ticks into the cycle, and the
    // second hit a full cycle of six behind it: eight ticks holds exactly one.
    utils::run_ticks(&mut app, 8);

    assert_eq!(
        utils::health(&app, target),
        10,
        "standing, the same gun lands its hit"
    );
}

/// An Attack order owns the gun for as long as it holds it: the cycle runs once
/// in six ticks, not twice, which is what two workers on one weapon would give.
#[test]
fn ordered_attack_fires_one_cycle_only() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 5), Some(0)).unwrap();
    let (target, target_id) =
        spawn::spawn_entity(world, "dummy", utils::pos(5, 3), Some(1)).unwrap();

    // Ordered onto a target its own initiative would have taken anyway, so both
    // paths want this fight and only one may have it.
    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::health(&app, target), 10, "one cycle, one hit");
}

/// A stance that picks no targets picks none while walking either.
#[test]
fn held_fire_rolls_past_without_shooting() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (_, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(11, 7), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(17, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::health(&app, target),
        20,
        "a stance that picks no targets picks none while walking either",
    );
}

/// An idle body's guns hunt for themselves, and hunting is not walking: a gun
/// notices what its body could never reach, comes round on it, and waits.
#[test]
fn idle_gun_hunts_what_it_cannot_reach_without_walking() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, _) = spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    // Six cells north: inside the range it engages at, outside the reach it fires
    // at, and nothing has ordered it anywhere.
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(5, 4), Some(1)).unwrap();

    utils::run_ticks(&mut app, 16);

    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::NORTH,
        "the gun came round on what it noticed",
    );
    assert_eq!(utils::health(&app, target), 20, "and held its fire");
    assert_eq!(
        utils::position_of(app.world(), wagon),
        utils::pos(5, 10),
        "and the body it sits on never stirred",
    );
}

/// A stance that picks no targets keeps every gun on the body still.
#[test]
fn held_fire_keeps_idle_guns_still() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (wagon, wagon_id) =
        spawn::spawn_entity(world, "rolling_gun", utils::pos(5, 10), Some(0)).unwrap();
    let (target, _) = spawn::spawn_entity(world, "dummy", utils::pos(5, 7), Some(1)).unwrap();

    utils::select(&mut app, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::HoldFire,
        },
    );
    utils::run_ticks(&mut app, 16);

    assert_eq!(
        utils::bearing_of(app.world(), wagon),
        Facing::SOUTH,
        "the gun never came round: it was left as it was mounted",
    );
    assert_eq!(utils::health(&app, target), 20, "and never fired");
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// How far into its cycle `entity`'s weapon is, as the gun that works it counts.
fn swing_of(app: &App, entity: Entity) -> u32 {
    app.world()
        .get::<TurretsComponent>(entity)
        .expect("a turreted entity carries the cycles its guns are on")
        .0[0]
        .phase
}
