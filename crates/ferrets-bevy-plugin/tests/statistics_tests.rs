//! Simulation events and the tallies folded from them: what the tick announces,
//! what counts toward a player's statistics and what deliberately does not, and
//! the record's per-tick lifetime.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::GameSet;

use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    repair::{RepairCost, RepairRate},
    skills::{EntityCastEffect, EntityCastTarget, SkillCaster, SkillDef},
    work::WorkPresence,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SkillCasterRef},
    components::resource::ResourceSourceComponent,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause, SpendCause},
    game_loop::damage,
    movement_model::MovementModel,
    resources::PlayerResources,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn,
    statistics::Statistics,
};

//
// ─── What the tick announces ────────────────────────────────────────────────
//

#[test]
fn placing_entity_announces_spawn() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, soldier) = spawn::spawn_entity(
        world,
        "soldier",
        utils::pos(4, 4),
        Some(0),
        SpawnCause::Placed,
    )
    .unwrap();

    let announced = world.resource::<EventRecord>().events().to_vec();
    let spawned = announced
        .iter()
        .find(|event| matches!(event, SimulationEvent::EntitySpawned { entity, .. } if *entity == soldier))
        .expect("a placement announces a spawn");
    assert!(matches!(
        spawned,
        SimulationEvent::EntitySpawned {
            cause: SpawnCause::Placed,
            ..
        }
    ));
}

#[test]
fn destroying_entity_announces_death_with_its_cause() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (entity, soldier) =
        spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();
    let (_, killer) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(1)).unwrap();
    world.resource_mut::<EventRecord>().clear();

    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(1),
        },
    );

    let announced = world.resource::<EventRecord>().events().to_vec();
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::EntityDied {
            entity,
            owner: Some(0),
            cause: DeathCause::Killed { by_owner: Some(1), .. },
            ..
        } if *entity == soldier
    )));
}

#[test]
fn record_holds_only_current_tick() {
    let mut app = utils::orders_app();
    spawn::spawn_entity(
        app.world_mut(),
        "soldier",
        utils::pos(4, 4),
        Some(0),
        SpawnCause::Placed,
    )
    .unwrap();
    assert!(
        !app.world().resource::<EventRecord>().events().is_empty(),
        "the placement is announced before any tick runs"
    );

    utils::run_ticks(&mut app, 1);

    assert!(
        app.world().resource::<EventRecord>().events().is_empty(),
        "a completed tick retires what it announced"
    );
}

//
// ─── Production ─────────────────────────────────────────────────────────────
//

#[test]
fn training_unit_counts_toward_production() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, barracks) =
        spawn::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks,
            type_name: "soldier".into(),
        },
    );
    utils::run_ticks(&mut app, 30);

    let soldier = type_id(&app, "soldier");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .produced(soldier),
        1
    );
}

#[test]
fn placed_entity_does_not_count_toward_production() {
    let mut app = utils::orders_app();
    // Announced deliberately: a silent fixture would make this pass without the
    // tally ever having to decide anything.
    spawn::spawn_entity(
        app.world_mut(),
        "soldier",
        utils::pos(4, 4),
        Some(0),
        SpawnCause::Placed,
    )
    .unwrap();
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .produced(soldier),
        0,
        "what the map handed a player was not produced by them"
    );
}

#[test]
fn entity_killed_same_tick_still_counts_as_produced() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, trainer) =
        spawn::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    let (entity, _) = spawn::spawn_entity(
        world,
        "soldier",
        utils::pos(4, 4),
        Some(0),
        SpawnCause::Trained { trainer },
    )
    .unwrap();
    let (_, killer) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(1)).unwrap();
    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(1),
        },
    );
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).produced(soldier),
        1,
        "made and lost inside one tick is still made"
    );
    assert_eq!(statistics.player(0).lost(soldier), 1);
}

#[test]
fn morphing_does_not_count_toward_production() {
    let mut app = utils::morph_app(MovementModel::Cell);
    let (_, whelp) = utils::create_owned(&mut app, "whelp", 5, 5, 0);

    utils::select(&mut app, whelp);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "giant".into(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).produced(type_id(&app, "giant")),
        0,
        "a change of form makes nothing new — a form that toggles would count every switch"
    );
    assert_eq!(statistics.player(0).produced(type_id(&app, "whelp")), 0);
}

#[test]
fn finished_building_counts_toward_production_naming_finisher() {
    let mut app = utils::orders_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
    let (_, worker) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 40);

    let depot = type_id(&app, "depot");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .produced(depot),
        1
    );
    assert_eq!(
        app.world().resource::<Announced>().founded(),
        vec![worker],
        "the site's spawn is announced when it is raised, naming who placed it"
    );
    assert_eq!(
        app.world().resource::<Announced>().completed(),
        vec![worker],
        "and the completion is announced once, naming who finished the work"
    );
}

#[test]
fn cancelled_site_does_not_count_toward_production() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    // Far enough for the site to have started and paid.
    utils::run_ticks(&mut app, 12);
    utils::stop_orders(app.world_mut(), worker);
    utils::run_ticks(&mut app, 2);

    let depot = type_id(&app, "depot");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .produced(depot),
        0,
        "a site given up on was never a finished building"
    );
}

//
// ─── Losses and kills ───────────────────────────────────────────────────────
//

#[test]
fn killed_entity_counts_as_loss_for_owner_and_kill_for_attacker() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();
    let (_, killer) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(1)).unwrap();

    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(1),
        },
    );
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(soldier), 1);
    assert_eq!(statistics.player(1).killed(soldier), 1);
}

#[test]
fn cancelled_entity_is_neither_loss_nor_kill() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();

    spawn::despawn_entity(world, entity, DeathCause::Cancelled);
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(soldier), 0);
    assert_eq!(statistics.player(1).killed(soldier), 0);
}

#[test]
fn killing_own_entity_counts_loss_without_crediting_kill() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();
    let (_, own_side) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(0)).unwrap();

    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: own_side,
            by_owner: Some(0),
        },
    );
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(soldier), 1);
    assert_eq!(
        statistics.player(0).killed(soldier),
        0,
        "shooting your own earns no kill"
    );
}

#[test]
fn allied_fire_counts_loss_without_kill_credit() {
    let mut app = utils::transport_app();
    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "rifleman", utils::pos(4, 4), Some(0)).unwrap();
    let (_, ally) = spawn::create_entity(world, "rifleman", utils::pos(6, 4), Some(1)).unwrap();

    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: ally,
            by_owner: Some(1),
        },
    );
    utils::run_ticks(&mut app, 1);

    let rifleman = type_id(&app, "rifleman");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(rifleman), 1);
    assert_eq!(
        statistics.player(1).killed(rifleman),
        0,
        "downing an ally's earns no kill"
    );
}

#[test]
fn allied_damage_is_taken_but_not_dealt() {
    let mut app = utils::transport_app();
    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "rifleman", utils::pos(4, 4), Some(0)).unwrap();
    let (_, ally) = spawn::create_entity(world, "rifleman", utils::pos(6, 4), Some(1)).unwrap();

    damage::apply(world, ally, entity, FixedU64::from_num(10));
    utils::run_ticks(&mut app, 1);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).damage_taken(), FixedU64::from_num(10));
    assert_eq!(
        statistics.player(1).damage_dealt(),
        FixedU64::ZERO,
        "splash on an ally is not damage dealt to enemies"
    );
}

#[test]
fn dying_attacker_keeps_kill_and_damage_credit() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (attacker_entity, attacker) =
        spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();
    let (victim, _) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(1)).unwrap();

    // The attacker starts dying, then a shot it already fired lands.
    spawn::despawn_entity(world, attacker_entity, DeathCause::Cancelled);
    damage::apply(world, attacker, victim, FixedU64::from_num(50));
    utils::run_ticks(&mut app, 1);

    let soldier = type_id(&app, "soldier");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).killed(soldier),
        1,
        "a shot outliving its shooter still credits the shooter's owner"
    );
    assert_eq!(statistics.player(0).damage_dealt(), FixedU64::from_num(50));
    assert_eq!(statistics.player(1).lost(soldier), 1);
}

#[test]
fn passengers_lost_with_killed_transport_count_for_both_sides() {
    let mut app = utils::transport_app();
    let (wagon_entity, wagon) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rider) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    let (_, enemy) = utils::create_owned(&mut app, "rifleman", 20, 20, 2);
    utils::send_to(&mut app, rider, wagon);
    utils::run_until_aboard(&mut app, wagon_entity, 1, 30);

    spawn::despawn_entity(
        app.world_mut(),
        wagon_entity,
        DeathCause::Killed {
            by: enemy,
            by_owner: Some(2),
        },
    );
    utils::run_ticks(&mut app, 1);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(type_id(&app, "wagon")), 1);
    assert_eq!(
        statistics.player(0).lost(type_id(&app, "rifleman")),
        1,
        "a passenger going down with its carrier is a loss"
    );
    assert_eq!(
        statistics.player(2).killed(type_id(&app, "rifleman")),
        1,
        "and a kill for whoever sank the carrier"
    );
}

#[test]
fn cancelled_transport_takes_passengers_without_loss_or_kill() {
    let mut app = utils::transport_app();
    let (wagon_entity, wagon) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rider) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rider, wagon);
    utils::run_until_aboard(&mut app, wagon_entity, 1, 30);

    spawn::despawn_entity(app.world_mut(), wagon_entity, DeathCause::Cancelled);
    utils::run_ticks(&mut app, 1);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).lost(type_id(&app, "rifleman")),
        0,
        "no enemy took it, so nobody lost it and nobody killed it"
    );
}

//
// ─── Damage ─────────────────────────────────────────────────────────────────
//

#[test]
fn weapon_damage_counts_for_dealer_and_taker() {
    let mut app = utils::combat_app();
    let (_, soldier) = utils::create_owned(&mut app, "soldier", 4, 6, 0);
    let (_, dummy) =
        spawn::create_entity(app.world_mut(), "dummy", utils::pos(5, 6), Some(1)).unwrap();

    utils::attack(&mut app, soldier, dummy);
    utils::run_ticks(&mut app, 30);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).damage_dealt(),
        FixedU64::from_num(20),
        "every landed hit adds up, to exactly what the target could take"
    );
    assert_eq!(statistics.player(1).damage_taken(), FixedU64::from_num(20));
}

#[test]
fn skill_damage_counts_like_weapon_hit() {
    let (mut app, smite) = enemy_skill_app();
    let world = app.world_mut();
    let (_, mage) = spawn::create_entity(world, "mage", utils::pos(4, 4), Some(0)).unwrap();
    let (_, victim) = spawn::create_entity(world, "victim", utils::pos(5, 4), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: smite,
            caster: SkillCasterRef::Entity(mage),
            target: Some(victim),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(
        statistics.player(0).damage_dealt(),
        FixedU64::from_num(5),
        "a skill's damage lands like a weapon's"
    );
    assert_eq!(statistics.player(1).damage_taken(), FixedU64::from_num(5));
}

//
// ─── Economy ────────────────────────────────────────────────────────────────
//

#[test]
fn training_charges_count_toward_spending() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, barracks) =
        spawn::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks,
            type_name: "soldier".into(),
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        app.world().resource::<Statistics>().player(0).spent("gold"),
        30,
        "the unit's whole price is what was spent"
    );
}

#[test]
fn seeding_stockpile_is_not_gathering() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 500);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .gathered("gold"),
        0,
        "a grant is not something a carrier banked"
    );
}

#[test]
fn cancelled_build_records_refund_beside_charge() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    // Far enough for the site to have started and paid.
    utils::run_ticks(&mut app, 12);
    assert_eq!(
        app.world().resource::<Statistics>().player(0).spent("gold"),
        50,
        "starting the site charges the depot's price"
    );

    utils::stop_orders(app.world_mut(), worker);
    utils::run_ticks(&mut app, 2);

    let tally = app.world().resource::<Statistics>().player(0);
    assert_eq!(
        tally.refunded("gold"),
        50,
        "cancelling gives the whole charge back, and says so"
    );
    assert_eq!(
        tally.spent("gold"),
        50,
        "the charge itself is not rewritten — net spending is the reader's subtraction"
    );
}

#[test]
fn charge_and_refund_name_same_reason() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);
    utils::stop_orders(app.world_mut(), worker);
    utils::run_ticks(&mut app, 2);

    let seen = app.world().resource::<Announced>();
    assert_eq!(
        seen.charged(),
        seen.refunded(),
        "cancelling names exactly the reason it was charged for, so the two net out"
    );
    assert!(
        matches!(seen.charged().as_slice(), [SpendCause::Construction { .. }]),
        "and that reason is the construction, once: {:?}",
        seen.charged()
    );
}

#[test]
fn banked_load_counts_toward_gathering() {
    let mut app = utils::orders_app();
    let (_, worker) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        spawn::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 5;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, worker);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 40);

    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .gathered("gold"),
        5,
        "what the carrier banked is what was gathered"
    );
}

#[test]
fn depleted_source_is_neither_loss_nor_kill() {
    let mut app = utils::orders_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
    let (_, worker) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        spawn::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 5;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, worker);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 40);

    assert!(
        app.world()
            .resource::<Announced>()
            .deaths()
            .iter()
            .any(|(entity, cause)| *entity == mine_id && matches!(cause, DeathCause::Depleted)),
        "running dry is announced as depletion"
    );
    let mine_type = type_id(&app, "mine");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .killed(mine_type),
        0,
        "mining out a node is nobody's kill"
    );
}

#[test]
fn repair_charges_name_their_target() {
    let mut app = repair_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
    let (hall, hall_id) = utils::create_owned(&mut app, "hall", 10, 10, 0);
    let (_, fixer) = utils::create_owned(&mut app, "fixer", 8, 10, 0);
    utils::wound(&mut app, hall, "40");
    utils::grant_gold(&mut app, 100);

    utils::select(&mut app, fixer);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: hall_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);

    assert!(
        app.world().resource::<Statistics>().player(0).spent("gold") > 0,
        "mending is billed as the work is done"
    );
    let charged = app.world().resource::<Announced>().charged();
    assert!(
        charged
            .iter()
            .any(|cause| matches!(cause, SpendCause::Repair { target } if *target == hall_id)),
        "each charge names what was mended: {charged:?}"
    );
}

#[test]
fn cast_paying_no_resources_announces_no_spend() {
    let (mut app, smite) = enemy_skill_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
    let world = app.world_mut();
    let (_, mage) = spawn::create_entity(world, "mage", utils::pos(4, 4), Some(0)).unwrap();
    let (_, victim) = spawn::create_entity(world, "victim", utils::pos(5, 4), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: smite,
            caster: SkillCasterRef::Entity(mage),
            target: Some(victim),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(
        app.world().resource::<Announced>().charged().is_empty(),
        "a cast that paid nothing is not a charge"
    );
}

//
// ─── Research ───────────────────────────────────────────────────────────────
//

#[test]
fn research_completion_counts_beside_its_price() {
    let mut app = utils::research_app();
    let (_, lab) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 30);
    let smithing = utils::research_id(&app, "smithing");

    utils::push_command(
        &mut app,
        PlayerCommand::StartResearch {
            researcher: lab,
            research: smithing,
        },
    );
    utils::run_ticks(&mut app, 20);

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).research_completed(), 1);
    assert_eq!(
        statistics.player(0).spent("gold"),
        30,
        "and its whole price was spent"
    );
}

//
// ─── The game's slot in the tick ────────────────────────────────────────────
//

#[test]
fn game_slot_sees_tick_already_tallied_and_not_yet_retired() {
    let mut app = utils::orders_app();
    app.init_resource::<Observed>();
    app.add_systems(FixedLast, observe_tick.in_set(GameSet));

    let world = app.world_mut();
    let (entity, _) = spawn::create_entity(world, "soldier", utils::pos(4, 4), Some(0)).unwrap();
    let (_, killer) = spawn::create_entity(world, "soldier", utils::pos(6, 4), Some(1)).unwrap();
    spawn::despawn_entity(
        world,
        entity,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(1),
        },
    );

    utils::run_ticks(&mut app, 1);

    let observed = app.world().resource::<Observed>();
    assert_eq!(
        observed.kills, 1,
        "the game's slot runs after the engine has tallied the tick"
    );
    assert!(
        observed.announcements > 0,
        "and before what the tick announced is retired"
    );
}

//
// ─── Skill casts ────────────────────────────────────────────────────────────
//

#[test]
fn skill_cast_names_entity_it_was_applied_to() {
    let (mut app, smite) = enemy_skill_app();
    let world = app.world_mut();
    let (_, mage) = spawn::create_entity(world, "mage", utils::pos(4, 4), Some(0)).unwrap();
    let (_, victim) = spawn::create_entity(world, "victim", utils::pos(5, 4), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: smite,
            caster: SkillCasterRef::Entity(mage),
            target: Some(victim),
        },
    );

    // Observed from inside the tick, since the record is retired at its end, and
    // run past the input delay a command waits out before it executes.
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        app.world().resource::<Announced>().casts(),
        vec![(mage, victim)],
        "the cast names the caster and what the skill landed on"
    );
}

#[test]
fn player_skill_cast_counts_toward_skills_cast() {
    let mut app = utils::player_effects_app();
    utils::create_owned(&mut app, "runner", 5, 5, 0);
    utils::grant_gold(&mut app, 30);
    utils::run_ticks(&mut app, 1);
    let drums = app
        .world()
        .resource::<ferrets_content::registry::ContentRegistry>()
        .skill("drums")
        .expect("drums is registered");

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: drums,
            caster: SkillCasterRef::Player,
            target: None,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        app.world().resource::<Statistics>().player(0).skills_cast(),
        1,
        "a player's own skill is a skill cast, not only an entity's"
    );
}

//
// ─── Off the map, and what is left behind ───────────────────────────────────
//

#[test]
fn remains_name_entity_they_are_left_by() {
    let mut app = utils::combat_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));

    let world = app.world_mut();
    let (dummy, dummy_id) =
        spawn::create_entity(world, "dummy", utils::pos(6, 6), Some(1)).unwrap();
    let (_, killer) = spawn::create_entity(world, "soldier", utils::pos(4, 6), Some(0)).unwrap();
    spawn::despawn_entity(
        world,
        dummy,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(0),
        },
    );
    // Past the dying phase, which is when the remains are left.
    utils::run_ticks(&mut app, 8);

    assert_eq!(
        app.world().resource::<Announced>().remains_of(),
        vec![dummy_id],
        "remains say whose they are, so a cue can be drawn without the corpse"
    );
}

#[test]
fn boarding_and_unloading_announce_going_off_map_and_back() {
    let mut app = utils::transport_app();
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));

    let (_, wagon) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rider) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);

    utils::select(&mut app, rider);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: wagon,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert!(
        app.world()
            .resource::<Announced>()
            .hidden()
            .contains(&rider),
        "boarding takes the passenger off the map, and says so"
    );

    utils::push_command(
        &mut app,
        PlayerCommand::Unload {
            transport: wagon,
            at: None,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert!(
        app.world()
            .resource::<Announced>()
            .revealed()
            .contains(&rider),
        "and unloading puts it back, and says that too"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// The type handle a name resolves to, for reading a per-type tally back.
fn type_id(app: &App, type_name: &str) -> ferrets_content::entity_type_def::EntityTypeId {
    app.world()
        .resource::<ferrets_content::registry::ContentRegistry>()
        .type_id(type_name)
        .expect("test content registers the type")
}

/// Two players, and a `mage` whose one skill strikes an enemy — so a cast names
/// something other than its caster, which a self-cast could never distinguish.
fn enemy_skill_app() -> (App, ferrets_content::skills::SkillId) {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let smite = {
        let mut registry = app
            .world_mut()
            .resource_mut::<ferrets_content::registry::ContentRegistry>();
        let smite = registry.register_skill(
            "smite",
            SkillDef {
                cooldown: 5,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Enemy,
                    effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("mage")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50)
                // A named target must be in sight for the cast to be allowed at
                // all, so the caster needs eyes.
                .with_sight_range(8)
                .with_skills([smite]),
        );
        registry.register(
            EntityTypeDef::new("victim")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50),
        );
        smite
    };
    app.world_mut()
        .resource::<ferrets_content::registry::ContentRegistry>()
        .validate();
    app.world_mut().resource_mut::<GameSession>().start();
    (app, smite)
}

/// One player, a damageable `hall` costing gold, and a `fixer` mending tagged
/// buildings at pro-rata cost — so a repair both spends and says why.
fn repair_app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app
            .world_mut()
            .resource_mut::<ferrets_content::registry::ContentRegistry>();
        registry.register_resource("gold");
        registry.register_tag("building");
        registry.register(
            EntityTypeDef::new("hall")
                .with_location(utils::GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_cost([("gold", 50)])
                .with_build_time(10)
                .with_tags(["building"]),
        );
        registry.register(
            EntityTypeDef::new("fixer")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_COST_FACTOR, FixedU64::ONE)
                .with_repairer(
                    ["building"],
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present,
                    false,
                    RepairCost::ProRata,
                    None,
                ),
        );
    }
    app.world_mut()
        .resource::<ferrets_content::registry::ContentRegistry>()
        .validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// What a system in [`GameSet`] saw while the tick it belongs to was still open.
#[derive(Resource, Default)]
struct Observed {
    kills: u32,
    announcements: usize,
}

/// Stands in for a game's own per-tick work: reads the tick it was handed and
/// remembers what was visible at that moment.
fn observe_tick(
    record: Res<EventRecord>,
    statistics: Res<Statistics>,
    registry: Res<ferrets_content::registry::ContentRegistry>,
    mut observed: ResMut<Observed>,
) {
    if let Some(soldier) = registry.type_id("soldier") {
        observed.kills = statistics.player(1).killed(soldier);
    }
    observed.announcements = record.events().len();
}

/// Everything the ticks announced, as a game system in [`GameSet`] saw it.
///
/// One recorder rather than one per subject: these tests assert what the events
/// *carry*, which the tallies deliberately drop — a spend's reason, a cast's
/// target, a corpse's origin — so each simply filters the log it wants.
#[derive(Resource, Default)]
struct Announced(Vec<SimulationEvent>);

impl Announced {
    /// The causes of every charge, in order.
    fn charged(&self) -> Vec<SpendCause> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::ResourcesSpent { cause, .. } => Some(*cause),
                _ => None,
            })
            .collect()
    }

    /// The causes of every give-back, in order.
    fn refunded(&self) -> Vec<SpendCause> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::ResourcesRefunded { cause, .. } => Some(*cause),
                _ => None,
            })
            .collect()
    }

    /// Every cast, as the caster and what the skill landed on.
    fn casts(&self) -> Vec<(SimulationId, SimulationId)> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::SkillCast { caster, target, .. } => Some((*caster, *target)),
                _ => None,
            })
            .collect()
    }

    /// The builder named by every founded construction site.
    fn founded(&self) -> Vec<SimulationId> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::EntitySpawned {
                    cause: SpawnCause::Founded { builder },
                    ..
                } => Some(*builder),
                _ => None,
            })
            .collect()
    }

    /// The builder named by every finished construction.
    fn completed(&self) -> Vec<SimulationId> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::ConstructionCompleted { builder, .. } => Some(*builder),
                _ => None,
            })
            .collect()
    }

    /// Every death, as the subject and why.
    fn deaths(&self) -> Vec<(SimulationId, DeathCause)> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::EntityDied { entity, cause, .. } => Some((*entity, *cause)),
                _ => None,
            })
            .collect()
    }

    /// The entities every set of remains was left by.
    fn remains_of(&self) -> Vec<SimulationId> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::EntitySpawned {
                    cause: SpawnCause::Remains { of },
                    ..
                } => Some(*of),
                _ => None,
            })
            .collect()
    }

    /// Every entity that went off the map.
    fn hidden(&self) -> Vec<SimulationId> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::EntityHidden { entity, .. } => Some(*entity),
                _ => None,
            })
            .collect()
    }

    /// Every entity that came back onto it.
    fn revealed(&self) -> Vec<SimulationId> {
        self.0
            .iter()
            .filter_map(|event| match event {
                SimulationEvent::EntityRevealed { entity, .. } => Some(*entity),
                _ => None,
            })
            .collect()
    }
}

/// A game system in [`GameSet`] keeping every announcement of every tick it ran
/// for, so a test can assert over the whole run rather than one tick of it.
fn note_announced(record: Res<EventRecord>, mut seen: ResMut<Announced>) {
    seen.0.extend(record.events().iter().cloned());
}
