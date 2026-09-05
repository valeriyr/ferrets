//! Projectile delivery: a shot lands after its flight time, its blast falls off by
//! band, and it outlives the unit that fired it.

use bevy::prelude::*;
use ferrets_content::{
    attack::{AttackDef, Delivery, Weapon},
    entity_type_def::EntityTypeDef,
    location::Solidity,
    projectile::{Aim, ProjectileDef},
    registry::ContentRegistry,
    splash::{SplashDef, SplashShape},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::location::LocationComponent,
    impacts::PendingImpacts,
    order::AttackTarget,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

mod utils;

//
// ─── Flight time ────────────────────────────────────────────────────────────
//

#[test]
fn shot_lands_after_its_flight_time() {
    let mut app = app();
    let (_, gunner) = utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 5), Some(0))
        .expect("gunner spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(9, 5), Some(1))
            .expect("target spawns");

    utils::attack(&mut app, gunner, target_id);
    // The command needs three ticks to arrive, then the shell crosses four cells at
    // half a cell per tick — eight more.
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::health(&app, target),
        100,
        "the shell is still in the air, so nothing has been hit yet"
    );
    assert_eq!(
        app.world().resource::<PendingImpacts>().in_flight().len(),
        1,
        "exactly one shot is in flight"
    );

    utils::run_ticks(&mut app, 8);
    // One shell has arrived, for the gunner's full 20 damage against an unarmored
    // target; the next is still in its attack cycle.
    assert_eq!(utils::health(&app, target), 80);
}

//
// ─── Blast falloff ──────────────────────────────────────────────────────────
//

#[test]
fn blast_damages_bystanders_by_band() {
    let mut app = app();
    let (_, gunner) = utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 20), Some(0))
        .expect("gunner spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(9, 20), Some(1))
            .expect("target spawns");
    // One cell from the impact (half damage) and two cells away (quarter damage).
    let (near, _) = utils::create_entity(app.world_mut(), "dummy", utils::pos(10, 20), Some(1))
        .expect("near bystander spawns");
    let (far, _) = utils::create_entity(app.world_mut(), "dummy", utils::pos(11, 20), Some(1))
        .expect("far bystander spawns");

    utils::attack(&mut app, gunner, target_id);
    utils::run_ticks(&mut app, 14);

    let (direct, near_lost, far_lost) = (
        100 - utils::health(&app, target),
        100 - utils::health(&app, near),
        100 - utils::health(&app, far),
    );
    // 20 damage at the impact, halved one cell out and quartered two cells out.
    assert_eq!((direct, near_lost, far_lost), (20, 10, 5));
}

#[test]
fn blast_spares_own_side_without_friendly_fire() {
    let mut app = app();
    let (_, gunner) = utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 26), Some(0))
        .expect("gunner spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(9, 26), Some(1))
            .expect("target spawns");
    // An own-side unit standing inside the blast.
    let (ally, _) = utils::create_entity(app.world_mut(), "dummy", utils::pos(10, 26), Some(0))
        .expect("ally spawns");

    utils::attack(&mut app, gunner, target_id);
    utils::run_ticks(&mut app, 14);

    // The enemy takes the direct 20; the gunner's own unit stands one cell from the
    // impact, inside the half-damage band, and is untouched.
    assert_eq!(
        (utils::health(&app, target), utils::health(&app, ally)),
        (80, 100)
    );
}

#[test]
fn blast_scales_bonus_and_subtracts_armor_in_full() {
    // An armored bystander in the half band. The band must scale the anti-armor
    // bonus along with the base — otherwise the blast edge deals its full bonus —
    // and armor must then come off in full, because it mitigates each hit it takes.
    let mut app = app();
    let (_, gunner) = utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 30), Some(0))
        .expect("gunner spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(9, 30), Some(1))
            .expect("target spawns");
    let (tank, _) = utils::create_entity(app.world_mut(), "tank", utils::pos(10, 30), Some(1))
        .expect("armored bystander spawns");

    utils::attack(&mut app, gunner, target_id);
    utils::run_ticks(&mut app, 14);

    // Direct hit on the untagged target: 20 base, no bonus, no armor.
    assert_eq!(100 - utils::health(&app, target), 20);
    // Bystander one cell out: (20 base + 12 vs armored) x 0.5 = 16, less 6 armor.
    assert_eq!(100 - utils::health(&app, tank), 10);
}

//
// ─── Cell-aimed shots ───────────────────────────────────────────────────────
//

#[test]
fn cell_aimed_shot_misses_target_that_moves_away() {
    // The shell is sent to the cell the target stood on. The target walks off before
    // it lands, so nothing is there to take the hit.
    let mut app = app();
    let (_, sieger) = utils::create_entity(app.world_mut(), "sieger", utils::pos(5, 4), Some(0))
        .expect("sieger spawns");
    let (runner, runner_id) =
        utils::create_entity(app.world_mut(), "runner", utils::pos(11, 4), Some(1))
            .expect("runner spawns");

    utils::attack(&mut app, sieger, runner_id);
    utils::run_ticks(&mut app, 6);
    // Step the runner off the aimed cell while the shell is still in the air. The
    // test is about where the hit resolves, so it moves the unit directly rather
    // than routing a move order.
    app.world_mut()
        .get_mut::<LocationComponent>(runner)
        .expect("the runner has a location")
        .position = utils::pos(20, 4);
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::health(&app, runner),
        60,
        "the shell landed on the cell the runner left"
    );
}

#[test]
fn cell_aimed_shot_hits_whoever_stands_on_cell() {
    // Same shot against a target that does not move: the cell is still occupied when
    // the shell arrives, so the occupant takes the full hit even with no blast.
    let mut app = app();
    let (_, sieger) = utils::create_entity(app.world_mut(), "sieger", utils::pos(5, 8), Some(0))
        .expect("sieger spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(11, 8), Some(1))
            .expect("target spawns");

    utils::attack(&mut app, sieger, target_id);
    utils::run_ticks(&mut app, 24);

    assert_eq!(100 - utils::health(&app, target), 20);
}

#[test]
fn cell_aiming_weapon_can_be_ordered_onto_bare_ground() {
    // Nothing is standing on the cell, so the shell simply lands. What matters is
    // that the order is accepted and the shot is released at all.
    let mut app = app();
    let (_, sieger) = utils::create_entity(app.world_mut(), "sieger", utils::pos(5, 14), Some(0))
        .expect("sieger spawns");

    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: sieger,
            mode: SelectMode::Replace,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Position(utils::pos(11, 14)),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 6);

    assert_eq!(
        app.world().resource::<PendingImpacts>().in_flight().len(),
        1,
        "the ground attack released a shell"
    );
}

#[test]
fn target_following_weapon_refuses_ground_order() {
    // The gunner's shells follow what they were fired at, so there is nothing for a
    // bare cell to mean and the order is dropped rather than silently reinterpreted.
    let mut app = app();
    let (gunner_entity, gunner) =
        utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 18), Some(0))
            .expect("gunner spawns");

    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: gunner,
            mode: SelectMode::Replace,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Position(utils::pos(11, 18)),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 6);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), gunner_entity),
        "the gunner took no order"
    );
    assert_eq!(
        app.world().resource::<PendingImpacts>().in_flight().len(),
        0,
        "and released nothing"
    );
}

//
// ─── Outliving the attacker ─────────────────────────────────────────────────
//

#[test]
fn shot_lands_after_its_attacker_dies() {
    let mut app = app();
    let (gunner_entity, gunner) =
        utils::create_entity(app.world_mut(), "gunner", utils::pos(5, 12), Some(0))
            .expect("gunner spawns");
    let (target, target_id) =
        utils::create_entity(app.world_mut(), "dummy", utils::pos(9, 12), Some(1))
            .expect("target spawns");

    utils::attack(&mut app, gunner, target_id);
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        app.world().resource::<PendingImpacts>().in_flight().len(),
        1,
        "the shot is in flight"
    );

    // Kill the gunner while its shell is still travelling.
    spawn::destroy_entity(app.world_mut(), gunner_entity);
    utils::run_ticks(&mut app, 8);

    // The gunner is gone, but its shell still deals the 20 damage frozen at release.
    assert_eq!(utils::health(&app, target), 80);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Two players and a roster with a `gunner` firing slow bursting shells, and a
/// stationary 100-hp `dummy` to catch them.
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_tag("armored");
        let shell = registry.register_projectile(
            "shell",
            ProjectileDef::new(FixedU64::from_num(0.5), Aim::Entity),
        );
        registry.register(
            EntityTypeDef::new("gunner")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(12)
                .with_health(40)
                .with_bonus_damage_vs([("armored", 12u32)])
                .with_attack(
                    AttackDef::new(Weapon::new(
                        utils::GROUND,
                        Delivery::Projectile(shell),
                        Some(SplashDef::new(
                            SplashShape::Circular,
                            vec![(1, FixedU64::from_num(0.5)), (2, FixedU64::from_num(0.25))],
                            utils::GROUND,
                            false,
                        )),
                    )),
                    20,
                    6,
                    6,
                    4,
                    2,
                ),
        );
        let lob = registry.register_projectile(
            "lob",
            ProjectileDef::new(FixedU64::from_num(0.5), Aim::Position),
        );
        registry.register(
            EntityTypeDef::new("sieger")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(12)
                .with_health(40)
                .with_attack(
                    AttackDef::new(Weapon::new(utils::GROUND, Delivery::Projectile(lob), None)),
                    20,
                    8,
                    8,
                    30,
                    2,
                ),
        );
        registry.register(
            EntityTypeDef::new("runner")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60),
        );
        registry.register(
            EntityTypeDef::new("tank")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(12)
                .with_health(100)
                .with_armor(6)
                .with_tags(["armored"]),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(100),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
