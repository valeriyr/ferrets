//! Fields end-to-end: sources grow and project while standing, coverage
//! decays by policy, placement reads the grid, effects fold into stats or
//! disable the entity, a disabled entity is gated at command, at the front of
//! its queue and mid-order, casts cover and clear, and a watched field is seen
//! by whoever covers it.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::game_view;
use ferrets_bevy_plugin::instantiate_map;
use ferrets_content::{
    attack::{Delivery, Weapon},
    build::BuilderAttendance,
    costs,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        FieldAction, FieldAffiliation, FieldCoverage, FieldDecay, FieldDef, FieldEffect,
        FieldEffectKind, FieldGrowth, FieldId, FieldPlacement, FieldSide, FieldSourceDef,
        FieldVision,
    },
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    research::ResearchDef,
    resource::{DepletionPolicy, HarvestData},
    skills::{EntityCastEffect, EntityCastTarget, SkillCaster, SkillDef},
    stats::{EntityModifier, ModifierOp},
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::WorkPresence,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_simulation::{
    checksum,
    command::{PlayerCommand, SkillCasterRef, SkillTarget},
    components::{
        research::ResearchComponent, resource::ResourceSourceComponent, train::TrainComponent,
    },
    entity_def::{self, Operation},
    fields::{self, FieldGrid},
    map_data::MapData,
    order::Order,
    player_research::PlayerResearch,
    requirements,
    session::{
        GameSession, ai_vision::AiVision, player_id::PlayerId, player_slot::PlayerSlot,
        player_type::PlayerType,
    },
    visibility::{CellVisibility, VisibilityGrid},
};

//
// ─── Sources ──────────────────────────────────────────────────────────────────
//

#[test]
fn instant_source_covers_its_radius_when_it_stands() {
    let mut app = field_app();
    let Fields { power, .. } = fields(&app);
    utils::create_owned(&mut app, "pylon", 10, 10, 0);

    utils::run_ticks(&mut app, 1);

    assert!(utils::covered_by(&app, power, 13, 10, 0));
    assert!(utils::covered_by(&app, power, 7, 10, 0));
    assert!(!utils::covered_by(&app, power, 14, 10, 0));
    assert!(!utils::covered_by(&app, power, 13, 10, 1));
}

#[test]
fn gradual_source_grows_one_cell_per_cycle_up_to_its_radius() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    utils::create_owned(&mut app, "hive", 10, 10, 0);

    utils::run_ticks(&mut app, 1);
    assert!(
        utils::covered_by(&app, creep, 12, 10, 0),
        "first ring at once"
    );
    assert!(!utils::covered_by(&app, creep, 13, 10, 0));

    utils::run_ticks(&mut app, 1);
    assert!(
        utils::covered_by(&app, creep, 13, 10, 0),
        "second ring after one cycle"
    );
    assert!(!utils::covered_by(&app, creep, 14, 10, 0));

    utils::run_ticks(&mut app, 10);
    assert!(utils::covered_by(&app, creep, 15, 10, 0), "radius reached");
    assert!(
        !utils::covered_by(&app, creep, 16, 10, 0),
        "never beyond the radius"
    );
}

#[test]
fn map_placed_source_stands_with_its_whole_field() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    utils::place(&mut app, "hive", 10, 10, 0);

    utils::run_ticks(&mut app, 1);

    assert!(utils::covered_by(&app, creep, 15, 10, 0));
}

#[test]
fn power_vanishes_on_tick_its_source_dies() {
    let mut app = field_app();
    let Fields { power, .. } = fields(&app);
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    utils::run_ticks(&mut app, 1);
    assert!(utils::covered_by(&app, power, 12, 10, 0));

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::covered_by(&app, power, 12, 10, 0));
    assert!(!utils::covered_by(&app, power, 10, 10, 0));
}

#[test]
fn creep_recedes_from_edge_inward_after_its_source_dies() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    let hive = utils::place(&mut app, "hive", 10, 10, 0);
    utils::run_ticks(&mut app, 1);
    assert!(utils::covered_by(&app, creep, 15, 10, 0));

    utils::deplete(&mut app, hive);
    utils::run_ticks(&mut app, 2);
    assert!(
        !utils::covered_by(&app, creep, 15, 10, 0),
        "outer ring gone first"
    );
    assert!(
        utils::covered_by(&app, creep, 12, 10, 0),
        "interior still stands"
    );

    utils::run_ticks(&mut app, 12);
    let grid = app.world().resource::<FieldGrid>();
    assert!(
        grid.cells(creep).all(|(_, mask)| mask.is_empty()),
        "nothing sustains it, so all of it recedes"
    );
}

#[test]
fn source_under_construction_projects_only_what_it_declares() {
    let mut app = field_app();
    let Fields { creep, power } = fields(&app);
    let builder = utils::create_owned(&mut app, "worker", 12, 13, 0).1;
    let electrician = utils::create_owned(&mut app, "worker", 20, 13, 0).1;
    build(&mut app, builder, "nest", 12, 10);
    build(&mut app, electrician, "pylon", 20, 10);
    // Long enough to raise both sites, far too short to finish either.
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "nest"), 1);

    assert!(
        utils::covered_by(&app, creep, 13, 10, 0),
        "the nest shows its patch"
    );
    assert!(
        !utils::covered_by(&app, creep, 14, 10, 0),
        "and does not grow yet"
    );
    assert!(
        !utils::covered_by(&app, power, 21, 10, 0),
        "the pylon projects nothing"
    );
}

#[test]
fn field_stops_at_terrain_its_layer_cannot_pass() {
    let mut app = field_app();
    let Fields { power, .. } = fields(&app);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_terrain("grass", utils::GROUND);
        registry.register_terrain("water", LayerMask::EMPTY);
    }
    let mut data = MapData::new("walled", Projection::Isometric, 16, 16);
    data.fill_terrain("grass");
    for y in 0..16 {
        data.set_terrain((12, y), "water");
    }
    instantiate_map(app.world_mut(), &data);
    utils::create_owned(&mut app, "pylon", 10, 8, 0);

    utils::run_ticks(&mut app, 1);

    assert!(utils::covered_by(&app, power, 11, 8, 0));
    assert!(
        !utils::covered_by(&app, power, 12, 8, 0),
        "water never carries it"
    );
    assert!(!utils::covered_by(&app, power, 13, 8, 0), "nor is it leapt");
}

//
// ─── Placement ────────────────────────────────────────────────────────────────
//

#[test]
fn placement_rules_read_footprint_anchor_and_affiliation() {
    let mut app = field_app();
    utils::place(&mut app, "hive", 10, 10, 0);
    utils::create_owned(&mut app, "pylon", 20, 10, 0);
    utils::run_ticks(&mut app, 1);

    assert!(allows(&app, 0, "spore", 13, 10), "on creep");
    assert!(!allows(&app, 0, "spore", 16, 10), "off creep");
    assert!(allows(&app, 1, "spore", 13, 10), "anyone's creep will do");
    assert!(!allows(&app, 0, "bunker", 13, 10), "forbidden on creep");
    assert!(allows(&app, 0, "bunker", 16, 10));
    assert!(
        allows(&app, 0, "spire", 14, 10),
        "whole footprint within reach"
    );
    assert!(
        !allows(&app, 0, "spire", 15, 10),
        "one footprint cell short"
    );
    assert!(
        allows(&app, 0, "gateway", 23, 10),
        "anchor powered, the rest not"
    );
    assert!(!allows(&app, 0, "gateway", 24, 10));
    assert!(
        !allows(&app, 1, "gateway", 21, 10),
        "another's power does not count"
    );
}

#[test]
fn build_command_raises_site_only_where_fields_allow() {
    let mut app = field_app();
    utils::place(&mut app, "hive", 10, 10, 0);
    let worker = utils::create_owned(&mut app, "worker", 14, 14, 0).1;

    build(&mut app, worker, "spore", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 30);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "spore"),
        0,
        "off creep"
    );

    build(&mut app, worker, "spore", 13, 12);
    utils::run_ticks(&mut app, utils::APPLY + 30);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "spore"),
        1,
        "on creep"
    );
}

#[test]
fn fizzled_interim_change_returns_to_origin_even_off_its_field() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    let hive = utils::place(&mut app, "hive", 10, 10, 0);
    utils::run_ticks(&mut app, 1);
    let (spore, spore_id) = utils::create_owned(&mut app, "spore", 13, 10, 0);
    // Standing where the tower's footprint would spread, so the landing is
    // refused when the change runs out.
    utils::create_owned(&mut app, "zergling", 12, 9, 1);

    utils::select(&mut app, spore_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "tower".into(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "pupa"), 1);

    // The hive dies while the pupa waits, and the creep under it recedes:
    // the spore could no longer be placed here. The refused landing still
    // returns to it.
    utils::deplete(&mut app, hive);
    utils::run_ticks(&mut app, 12);
    assert!(!covered_by_anyone(&app, creep, 13, 10), "creep is gone");
    assert_eq!(utils::count_of_type(app.world_mut(), "tower"), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "pupa"), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "spore"), 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), spore));
}

#[test]
fn morph_landing_on_forbidden_field_fizzles() {
    let mut app = field_app();
    utils::place(&mut app, "hive", 10, 10, 0);
    let (_, on_creep) = utils::create_owned(&mut app, "lander", 13, 10, 0);
    let (_, off_creep) = utils::create_owned(&mut app, "lander", 20, 20, 0);
    utils::run_ticks(&mut app, 1);

    for lander in [on_creep, off_creep] {
        utils::select(&mut app, lander);
        utils::push_command(
            &mut app,
            PlayerCommand::Morph {
                type_name: "bunker".into(),
                flush: true,
            },
        );
        utils::run_ticks(&mut app, utils::APPLY + 2);
    }

    assert_eq!(utils::count_of_type(app.world_mut(), "lander"), 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "bunker"), 1);
    assert_eq!(utils::owned_of_type(app.world_mut(), "bunker", 0).len(), 1);
    let bunker = utils::single_owned_of_type(app.world_mut(), "bunker", 0);
    assert_eq!(utils::cell_of(app.world(), bunker), CellPos::new(20, 20));
}

//
// ─── Effects ──────────────────────────────────────────────────────────────────
//

#[test]
fn inside_effect_folds_into_stats_where_entity_stands() {
    let mut app = field_app();
    utils::place(&mut app, "hive", 10, 10, 0);
    let on_creep = utils::create_owned(&mut app, "zergling", 13, 10, 1).0;
    let off_creep = utils::create_owned(&mut app, "zergling", 20, 20, 1).0;

    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::effective_speed(&app, on_creep), FixedU64::ONE);
    assert_eq!(
        utils::effective_speed(&app, off_creep),
        FixedU64::from_num(0.5)
    );
}

#[test]
fn own_field_modifier_ignores_rival_power() {
    let mut app = field_app();
    let acolyte = utils::create_owned(&mut app, "acolyte", 12, 10, 0).0;
    let rival_pylon = utils::create_owned(&mut app, "pylon", 10, 10, 1).0;
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_speed(&app, acolyte),
        FixedU64::from_num(0.5),
        "a rival's power is not its own"
    );

    utils::deplete(&mut app, rival_pylon);
    utils::create_owned(&mut app, "pylon", 10, 12, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::effective_speed(&app, acolyte), FixedU64::ONE);
}

#[test]
fn checksum_ignores_field_coverage() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    let spew = app
        .world()
        .resource::<ContentRegistry>()
        .skill("spew")
        .unwrap();
    utils::run_ticks(&mut app, 1);
    let before = checksum::state_checksum(app.world());

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: spew,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(24, 24))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(
        utils::covered_by(&app, creep, 24, 24, 0),
        "the patch landed"
    );
    assert_eq!(checksum::state_checksum(app.world()), before);
}

#[test]
fn outside_effect_drains_health_each_tick() {
    let mut app = field_app();
    utils::place(&mut app, "hive", 10, 10, 0);
    let sheltered = utils::create_owned(&mut app, "larva", 13, 10, 0).0;
    let exposed = utils::create_owned(&mut app, "larva", 20, 20, 0).0;

    utils::run_ticks(&mut app, 3);

    assert_eq!(
        utils::current_health(&app, sheltered),
        FixedU64::from_num(20)
    );
    assert_eq!(utils::current_health(&app, exposed), FixedU64::from_num(17));
}

#[test]
fn disabled_trainer_queues_commands_and_holds_queue_until_powered_again() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (gateway, gateway_id) = utils::create_owned(&mut app, "gateway", 12, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: gateway_id,
            type_name: "zealot".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::train_queue_len(app.world(), gateway), 1);
    // Two ticks of the four the zealot takes: the tick the command landed and
    // the one after.
    assert_eq!(training_progress(&app, gateway), 2);

    // Unpowered mid-training: the entry neither advances nor is lost, and its
    // progress stands where it was.
    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "zealot"), 0);
    assert_eq!(utils::train_queue_len(app.world(), gateway), 1);
    assert_eq!(training_progress(&app, gateway), 2);

    // Unpowered, a new command still queues and waits with the first.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: gateway_id,
            type_name: "zealot".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::train_queue_len(app.world(), gateway), 2);
    assert_eq!(utils::count_of_type(app.world_mut(), "zealot"), 0);

    // Powered again, the held entry finishes in the two ticks it had left,
    // not four from scratch, and the second follows four ticks later.
    utils::create_owned(&mut app, "pylon", 10, 12, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "zealot"), 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "zealot"), 1);
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "zealot"), 2);
}

#[test]
fn disabled_cannon_does_not_fire() {
    let mut app = field_app();
    utils::create_owned(&mut app, "cannon", 12, 10, 0);
    let dummy = utils::create_owned(&mut app, "dummy", 14, 10, 1).0;

    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::health(&app, dummy), 100, "unpowered, it holds fire");

    utils::create_owned(&mut app, "pylon", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    // Two-tick volleys of 10, the first landing a tick after acquisition:
    // nine hits in twenty ticks.
    assert_eq!(utils::health(&app, dummy), 10, "powered, it fights");
}

#[test]
fn disabled_prerequisite_still_satisfies_requirements() {
    let mut app = field_app();
    utils::create_owned(&mut app, "gateway", 12, 10, 0);
    utils::run_ticks(&mut app, 1);

    assert!(requirements::met(app.world(), 0, &["gateway".to_string()]));
}

//
// ─── Gates ──────────────────────────────────────────────────────────────────
//

#[test]
fn disabled_cannon_refuses_attack_command() {
    let mut app = field_app();
    let (cannon, cannon_id) = utils::create_owned(&mut app, "cannon", 12, 10, 0);
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 14, 10, 1);
    utils::run_ticks(&mut app, 1);

    utils::attack(&mut app, cannon_id, dummy_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), cannon),
        "unpowered, the order is refused"
    );

    utils::create_owned(&mut app, "pylon", 10, 10, 0);
    utils::attack(&mut app, cannon_id, dummy_id);
    utils::run_ticks(&mut app, utils::APPLY + 8);
    // Five two-tick volleys of 10 land in the eight ticks after the command
    // takes effect.
    assert_eq!(
        utils::health(&app, dummy),
        50,
        "powered, it takes the order"
    );
}

#[test]
fn attacking_cannon_losing_power_drops_its_target_in_one_tick() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (cannon, cannon_id) = utils::create_owned(&mut app, "cannon", 12, 10, 0);
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 14, 10, 1);
    utils::run_ticks(&mut app, 1);
    utils::attack(&mut app, cannon_id, dummy_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!utils::order_queue_is_empty(app.world_mut(), cannon));

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), cannon),
        "the attack is cancelled the tick power goes"
    );
    let health = utils::health(&app, dummy);
    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::health(&app, dummy),
        health,
        "and nothing fires after"
    );
}

#[test]
fn disabled_gateway_refuses_morph_command() {
    let mut app = field_app();
    let (gateway, gateway_id) = utils::create_owned(&mut app, "gateway", 12, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::select(&mut app, gateway_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "warpgate".into(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 12);

    assert!(utils::order_queue_is_empty(app.world_mut(), gateway));
    assert_eq!(utils::count_of_type(app.world_mut(), "warpgate"), 0);
}

#[test]
fn morphing_gateway_losing_power_still_lands() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (gateway, gateway_id) = utils::create_owned(&mut app, "gateway", 12, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::select(&mut app, gateway_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "warpgate".into(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(!utils::order_queue_is_empty(app.world_mut(), gateway));

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::count_of_type(app.world_mut(), "warpgate"), 1);
}

#[test]
fn change_under_way_lands_while_queued_change_is_swept() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (gateway, gateway_id) = utils::create_owned(&mut app, "gateway", 12, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::select(&mut app, gateway_id);
    for flush in [true, false] {
        utils::push_command(
            &mut app,
            PlayerCommand::Morph {
                type_name: "warpgate".into(),
                flush,
            },
        );
    }
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(entity_def::orders(app.world(), gateway).len(), 2);

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);
    assert!(
        matches!(
            entity_def::orders(app.world(), gateway).as_slice(),
            [Order::Morph { .. }]
        ),
        "the queued change is swept and leaves the one under way alone"
    );

    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "warpgate"), 1);
}

#[test]
fn disabled_probe_refuses_move_command() {
    let mut app = field_app();
    let (probe, probe_id) = utils::create_owned(&mut app, "probe", 20, 20, 0);
    utils::run_ticks(&mut app, 1);

    utils::select(&mut app, probe_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(24, 20),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);

    assert!(utils::order_queue_is_empty(app.world_mut(), probe));
    assert_eq!(utils::cell_of(app.world(), probe), CellPos::new(20, 20));
}

#[test]
fn probe_losing_power_drops_queued_moves_in_one_tick() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (probe, probe_id) = utils::create_owned(&mut app, "probe", 8, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::select(&mut app, probe_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 10),
            flush: true,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 11),
            flush: false,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!utils::order_queue_is_empty(app.world_mut(), probe));

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), probe),
        "both walks are cancelled together"
    );
}

#[test]
fn building_probe_losing_power_finishes_its_site() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let probe_id = utils::create_owned(&mut app, "probe", 11, 10, 0).1;
    utils::run_ticks(&mut app, 1);
    build(&mut app, probe_id, "gateway", 12, 11);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::count_of_type(app.world_mut(), "gateway"), 1);
    assert!(
        !requirements::met(app.world(), 0, &["gateway".to_string()]),
        "the site is still going up"
    );

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 6);

    assert!(
        requirements::met(app.world(), 0, &["gateway".to_string()]),
        "the frozen probe raised it to the end"
    );
}

#[test]
fn build_under_way_completes_while_queued_build_is_swept() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (probe, probe_id) = utils::create_owned(&mut app, "probe", 11, 10, 0);
    utils::run_ticks(&mut app, 1);
    // A site raised and being worked, with a second build queued behind it.
    build(&mut app, probe_id, "gateway", 12, 11);
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: probe_id,
            type_name: "gateway".into(),
            position: utils::pos(12, 14),
            flush: false,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::count_of_type(app.world_mut(), "gateway"), 1);
    assert_eq!(entity_def::orders(app.world(), probe).len(), 2);

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);
    assert!(
        matches!(
            entity_def::orders(app.world(), probe).as_slice(),
            [Order::Build { .. }]
        ),
        "the queued build, with nothing raised, is swept; the one under way stays"
    );

    // The first site finishes and nothing else is raised.
    utils::run_ticks(&mut app, 6);
    assert!(utils::order_queue_is_empty(app.world_mut(), probe));
    assert_eq!(utils::count_of_type(app.world_mut(), "gateway"), 1);
    assert!(requirements::met(app.world(), 0, &["gateway".to_string()]));
}

#[test]
fn disabled_researcher_queues_research_and_holds_progress_until_powered_again() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (library, library_id) = utils::create_owned(&mut app, "library", 12, 10, 0);
    let lore = utils::research_id(&app, "lore");
    utils::run_ticks(&mut app, 1);

    utils::push_command(
        &mut app,
        PlayerCommand::StartResearch {
            researcher: library_id,
            research: lore,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // Two ticks of the ten: the tick the command landed and the one after.
    assert_eq!(research_progress(&app, library), 2);

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 20);
    assert_eq!(research_progress(&app, library), 2, "held, not lost");
    assert!(
        !app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, lore)
    );

    // Powered again, the eight ticks left finish it.
    utils::create_owned(&mut app, "pylon", 10, 12, 0);
    utils::run_ticks(&mut app, 7);
    assert!(
        !app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, lore)
    );
    utils::run_ticks(&mut app, 1);
    assert!(
        app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, lore)
    );
}

#[test]
fn disabled_hangar_refuses_load_and_unload() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (hangar, hangar_id) = utils::create_owned(&mut app, "hangar", 12, 10, 0);
    let (_, rider_id) = utils::create_owned(&mut app, "zealot", 10, 12, 0);
    let (_, waiting_id) = utils::create_owned(&mut app, "zealot", 10, 13, 0);
    utils::run_ticks(&mut app, 1);
    utils::select(&mut app, rider_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: hangar_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, hangar, 1, 40);

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);

    utils::push_command(
        &mut app,
        PlayerCommand::Unload {
            transport: hangar_id,
            at: None,
            flush: true,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            transport: hangar_id,
            target: waiting_id,
            flush: false,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), hangar),
        "a dark hangar neither opens its door nor fetches anyone"
    );
    assert_eq!(utils::passengers_of(app.world(), hangar).len(), 1);
}

#[test]
fn disabled_battery_gun_stays_idle() {
    let mut app = field_app();
    utils::create_owned(&mut app, "battery", 12, 10, 0);
    let dummy = utils::create_owned(&mut app, "dummy", 14, 10, 1).0;

    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::health(&app, dummy),
        100,
        "unpowered, the gun is idle"
    );

    utils::create_owned(&mut app, "pylon", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    // Two-tick volleys of 10, the first landing a tick after acquisition:
    // nine hits in twenty ticks.
    assert_eq!(utils::health(&app, dummy), 10, "powered, the gun works");
}

#[test]
fn script_view_reads_disabled_structure() {
    let mut app = field_app();
    let gateway_id = utils::create_owned(&mut app, "gateway", 12, 10, 0).1;
    utils::run_ticks(&mut app, 1);
    let gateway_view = |app: &App| {
        game_view(app.world(), 0, "conclave", AiVision::Omniscient)
            .my_entities
            .into_iter()
            .find(|entity| entity.id == gateway_id.0)
            .expect("the gateway is the player's")
    };
    assert!(
        gateway_view(&app).disabled,
        "dark, the brain reads it disabled"
    );

    utils::create_owned(&mut app, "pylon", 10, 10, 0);
    utils::run_ticks(&mut app, 1);
    assert!(!gateway_view(&app).disabled, "powered, it does not");
}

#[test]
fn walking_builder_losing_power_gives_up_its_build() {
    let mut app = field_app();
    let pylon = utils::create_owned(&mut app, "pylon", 10, 10, 0).0;
    let (probe, probe_id) = utils::create_owned(&mut app, "probe", 8, 10, 0);
    utils::run_ticks(&mut app, 1);
    build(&mut app, probe_id, "pylon", 20, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!utils::order_queue_is_empty(app.world_mut(), probe));

    utils::deplete(&mut app, pylon);
    utils::run_ticks(&mut app, 1);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), probe),
        "a build with no site raised is cancelled with the walk"
    );
    // The depleted pylon lingers its one dying tick; no site takes its place.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "pylon"), 0);
}

#[test]
fn cannon_site_neither_fires_nor_takes_attack_orders() {
    let mut app = field_app();
    utils::create_owned(&mut app, "pylon", 10, 10, 0);
    let probe_id = utils::create_owned(&mut app, "probe", 11, 10, 0).1;
    let (dummy, dummy_id) = utils::create_owned(&mut app, "dummy", 14, 12, 1);
    utils::run_ticks(&mut app, 1);
    build(&mut app, probe_id, "cannon", 12, 12);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let site = utils::single_owned_of_type(app.world_mut(), "cannon", 0);
    let site_id = entity_def::simulation_id(app.world(), site);

    utils::attack(&mut app, site_id, dummy_id);
    utils::run_ticks(&mut app, utils::APPLY + 6);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), site),
        "a site takes no orders"
    );
    assert_eq!(utils::health(&app, dummy), 100, "and fires nothing");

    utils::run_ticks(&mut app, 20);
    // Of the 2·APPLY + 28 ticks since the build command, the site took its
    // twenty and acquisition its one; four two-tick volleys of 10 land in
    // the rest.
    assert_eq!(utils::health(&app, dummy), 60, "finished, it fights");
}

#[test]
fn probe_mends_disabled_gateway() {
    let mut app = field_app();
    utils::create_owned(&mut app, "pylon", 8, 10, 0);
    let probe_id = utils::create_owned(&mut app, "probe", 10, 10, 0).1;
    let (gateway, gateway_id) = utils::create_owned(&mut app, "gateway", 13, 10, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::operation(app.world(), gateway),
        Operation::Disabled,
        "the gateway stands past the pylon's reach"
    );
    utils::wound(&mut app, gateway, "40");

    utils::select(&mut app, probe_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: gateway_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 12);

    assert_eq!(
        utils::health(&app, gateway),
        100,
        "dark, it is mended all the same"
    );
}

#[test]
fn probe_sent_to_disabled_crystal_follows_it_instead_of_harvesting() {
    let mut app = field_app();
    utils::create_owned(&mut app, "pylon", 8, 10, 0);
    // The probe has no sight of its own; the cannon keeps the crystal in view.
    utils::create_owned(&mut app, "cannon", 12, 13, 0);
    let (probe, probe_id) = utils::create_owned(&mut app, "probe", 10, 10, 0);
    let (crystal, crystal_id) =
        utils::create_entity(app.world_mut(), "crystal", utils::pos(15, 10), None)
            .expect("the crystal fits");
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(crystal)
        .unwrap()
        .amount = 50;
    utils::run_ticks(&mut app, 1);

    utils::send_to(&mut app, probe_id, crystal_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let orders = entity_def::orders(app.world(), probe);
    assert!(
        orders
            .iter()
            .any(|order| matches!(order, Order::Follow { .. })),
        "a dark crystal yields nothing, so the click falls through to following it"
    );
    assert!(
        !orders
            .iter()
            .any(|order| matches!(order, Order::Harvest { .. })),
        "and no harvest is queued"
    );

    // Powers the crystal and the probe's approach alike.
    utils::create_owned(&mut app, "pylon", 13, 11, 0);
    utils::send_to(&mut app, probe_id, crystal_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(
        entity_def::orders(app.world(), probe)
            .iter()
            .any(|order| matches!(order, Order::Harvest { .. })),
        "powered, it is worked"
    );
}

#[test]
fn zealot_refuses_to_board_disabled_hangar() {
    let mut app = field_app();
    let (zealot, zealot_id) = utils::create_owned(&mut app, "zealot", 10, 10, 0);
    let (hangar, hangar_id) = utils::create_owned(&mut app, "hangar", 14, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::select(&mut app, zealot_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: hangar_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), zealot),
        "a dark hangar opens no door"
    );

    utils::create_owned(&mut app, "pylon", 14, 12, 0);
    utils::select(&mut app, zealot_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: hangar_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, hangar, 1, 40);
}

//
// ─── Casts ──────────────────────────────────────────────────────────────────
//

#[test]
fn position_cast_covers_cells_that_then_decay_unsustained() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    let spew = app
        .world()
        .resource::<ContentRegistry>()
        .skill("spew")
        .unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: spew,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(24, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(utils::covered_by(&app, creep, 24, 20, 0));
    assert!(utils::covered_by(&app, creep, 25, 20, 0));
    assert!(!utils::covered_by(&app, creep, 26, 20, 0));

    utils::run_ticks(&mut app, 6);
    assert!(
        !covered_by_anyone(&app, creep, 24, 20),
        "nothing sustains it"
    );
}

#[test]
fn position_cast_clears_orphaned_coverage() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    let hive = utils::place(&mut app, "hive", 10, 10, 0);
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    let scour = app
        .world()
        .resource::<ContentRegistry>()
        .skill("scour")
        .unwrap();
    utils::run_ticks(&mut app, 1);

    // Sustained creep shrugs the cast off.
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: scour,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(13, 10))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(utils::covered_by(&app, creep, 13, 10, 0));

    // Orphaned creep goes.
    utils::deplete(&mut app, hive);
    utils::run_ticks(&mut app, 1);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: scour,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(11, 10))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(!covered_by_anyone(&app, creep, 11, 10));
    assert!(!covered_by_anyone(&app, creep, 13, 10));
}

//
// ─── Vision ─────────────────────────────────────────────────────────────────
//

#[test]
fn watching_field_reveals_covered_cells_to_whoever_covers_them() {
    let mut app = field_app();
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    let spew = app
        .world()
        .resource::<ContentRegistry>()
        .skill("spew")
        .unwrap();

    // Nothing of player 0's sees out to (24, 20) before the creep gets there.
    utils::run_ticks(&mut app, 1);
    assert_eq!(seen_by(&app, 0, 24, 20), CellVisibility::Unexplored);

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: spew,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(24, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The spreader sees its creep; the rival, with no creep there, does not.
    assert_eq!(seen_by(&app, 0, 24, 20), CellVisibility::Visible);
    assert_eq!(seen_by(&app, 1, 24, 20), CellVisibility::Unexplored);

    // Once the creep has receded the cell is only remembered.
    utils::run_ticks(&mut app, 8);
    assert_eq!(seen_by(&app, 0, 24, 20), CellVisibility::Explored);
}

#[test]
fn standing_source_keeps_its_creep_in_sight() {
    let mut app = field_app();
    let Fields { creep, .. } = fields(&app);
    // The hive has no sight of its own; its creep is what watches.
    utils::place(&mut app, "hive", 10, 10, 0);
    utils::run_ticks(&mut app, 12);

    assert!(utils::covered_by(&app, creep, 14, 10, 0));
    assert_eq!(seen_by(&app, 0, 14, 10), CellVisibility::Visible);
    assert_eq!(seen_by(&app, 1, 14, 10), CellVisibility::Unexplored);
    assert!(!covered_by_anyone(&app, creep, 17, 10));
    assert_eq!(seen_by(&app, 0, 17, 10), CellVisibility::Unexplored);
}

#[test]
fn ally_sees_through_watched_field_and_enemy_does_not() {
    let mut app = field_app_with(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(2, PlayerType::Human, None, Some(2)),
    ]);
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    let spew = app
        .world()
        .resource::<ContentRegistry>()
        .skill("spew")
        .unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: spew,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(24, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The creep is player 0's alone, so only player 0's own grid is stamped;
    // the ally reads it through team vision, the enemy never does.
    assert_eq!(seen_by(&app, 1, 24, 20), CellVisibility::Unexplored);
    assert!(team_sees(&app, 0, 24, 20));
    assert!(team_sees(&app, 1, 24, 20));
    assert!(!team_sees(&app, 2, 24, 20));
}

#[test]
fn field_without_vision_reveals_nothing() {
    let mut app = field_app();
    let Fields { power, .. } = fields(&app);
    utils::place(&mut app, "pylon", 10, 10, 0);
    utils::run_ticks(&mut app, utils::APPLY);

    // Powered, but the pylon has no sight and power watches nothing.
    assert!(utils::covered_by(&app, power, 12, 10, 0));
    assert_eq!(seen_by(&app, 0, 12, 10), CellVisibility::Unexplored);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// The registered field handles, in the order the fixture registers them.
struct Fields {
    creep: FieldId,
    power: FieldId,
}

/// The fixture's field handles, read back from the registry.
fn fields(app: &App) -> Fields {
    let registry = app.world().resource::<ContentRegistry>();
    Fields {
        creep: registry.field("creep").unwrap(),
        power: registry.field("power").unwrap(),
    }
}

/// A one-cell walker with a little health and no sight of its own.
fn mover(name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_health(20)
        .with_dying(1, None)
}

/// A square structure of `side` cells that takes `build_time` ticks to raise.
fn building(name: &str, side: u32, build_time: u32) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(utils::GROUND, CellSize::new(side, side), Solidity::Solid)
        .with_health(100)
        .with_dying(1, None)
        .with_build_time(build_time)
}

/// One stat modifier, its magnitude as a decimal string.
fn modifier(stat: EntityStatId, op: ModifierOp, magnitude: &str) -> EntityModifier {
    EntityModifier {
        stat,
        op,
        magnitude: utils::signed_fixed(magnitude),
    }
}

/// App with two rival players and field content: a creep field that grows,
/// recedes and is watched by whoever spreads it, and a power field that appears
/// and vanishes at once and watches nothing.
fn field_app() -> App {
    field_app_with(utils::human_slots(2))
}

/// [`field_app`] over the given seats, for suites that need teams.
fn field_app_with(slots: Vec<PlayerSlot>) -> App {
    let mut app = utils::make_app(slots);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let creep = registry.register_field(
            "creep",
            FieldDef::new(
                utils::GROUND,
                FieldDecay::Gradual { cycle: 2 },
                FieldVision::Watched,
            ),
        );
        let power = registry.register_field(
            "power",
            FieldDef::new(utils::GROUND, FieldDecay::Instant, FieldVision::Dark),
        );

        // Creep sources: a hive that grows from one ring outward and a nest
        // that shows a patch while still going up.
        registry.register(
            building("hive", 2, 4).with_field_sources([FieldSourceDef::new(
                creep,
                4,
                FieldGrowth::Gradual {
                    cycle: 2,
                    initial_radius: 1,
                },
                None,
            )]),
        );
        registry.register(
            building("nest", 1, 6).with_field_sources([FieldSourceDef::new(
                creep,
                3,
                FieldGrowth::Gradual {
                    cycle: 1,
                    initial_radius: 1,
                },
                Some(1),
            )]),
        );
        // Creep readers: a spore that needs creep under its whole footprint, a
        // spire that needs it under a 2×2 one, a bunker that refuses it.
        // The spore may also grow into a tower through a pupa: a form change
        // that starts on creep and may end after the creep is gone.
        registry.register(
            building("spore", 1, 2)
                .with_field_placement([FieldPlacement::Requires {
                    field: creep,
                    of: FieldAffiliation::Anyone,
                    coverage: FieldCoverage::Footprint,
                }])
                .with_morphs([MorphTransition::new(
                    "tower",
                    Some("pupa"),
                    MorphTime::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Refundable,
                    Vec::new(),
                    Vec::<String>::new(),
                )]),
        );
        registry.register(building("pupa", 1, 2));
        registry.register(building("tower", 3, 2));
        registry.register(building("spire", 2, 2).with_field_placement([
            FieldPlacement::Requires {
                field: creep,
                of: FieldAffiliation::Anyone,
                coverage: FieldCoverage::Footprint,
            },
        ]));
        registry.register(
            building("bunker", 1, 2)
                .with_field_placement([FieldPlacement::Forbids { field: creep }]),
        );
        registry.register(building("lander", 1, 2).with_morphs([MorphTransition::new(
            "bunker",
            None,
            MorphTime::Constant(1),
            MorphPlacement::Revalidate,
            MorphCancel::Forfeit,
            Vec::new(),
            Vec::<String>::new(),
        )]));
        // Creep effects: a zergling twice as fast on anyone's creep, a larva
        // that withers off it.
        registry.register(mover("zergling").with_field_effects([FieldEffect::new(
            creep,
            FieldAffiliation::Anyone,
            FieldSide::Inside,
            FieldEffectKind::Modifiers(vec![modifier(
                EntityStatId::SPEED,
                ModifierOp::PercentAdd,
                "1.0",
            )]),
        )]));
        registry.register(
            EntityTypeDef::new("larva")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(1, None)
                // Modifiers move only the stats a type carries, so the drain
                // is declared at zero for the field to raise.
                .with_stat(EntityStatId::HEALTH_DRAIN, FixedU64::ZERO)
                .with_field_effects([FieldEffect::new(
                    creep,
                    FieldAffiliation::Anyone,
                    FieldSide::Outside,
                    FieldEffectKind::Modifiers(vec![modifier(
                        EntityStatId::HEALTH_DRAIN,
                        ModifierOp::FlatAdd,
                        "1",
                    )]),
                )]),
        );

        // Power: a pylon, and structures that need its owner's power under
        // their anchor and stand disabled outside it. The gateway may turn into
        // a warpgate; a probe raises and mends them, gathers crystal, and is
        // itself frozen off its owner's power; a hangar shelters zealots.
        registry.register_tag("structure");
        registry.register_resource("crystal");
        let unpowered_idles = || {
            FieldEffect::new(
                power,
                FieldAffiliation::Own,
                FieldSide::Outside,
                FieldEffectKind::Disabled,
            )
        };
        registry.register(
            building("pylon", 1, 8).with_field_sources([FieldSourceDef::new(
                power,
                3,
                FieldGrowth::Instant,
                None,
            )]),
        );
        registry.register(
            mover("zealot")
                .with_train_time(4)
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
        );
        registry.register(
            building("gateway", 2, 4)
                .with_tags(["structure"])
                .with_trainer(["zealot"])
                .with_field_placement([FieldPlacement::Requires {
                    field: power,
                    of: FieldAffiliation::Own,
                    coverage: FieldCoverage::Anchor,
                }])
                .with_morphs([MorphTransition::new(
                    "warpgate",
                    None,
                    MorphTime::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Forfeit,
                    Vec::new(),
                    Vec::<String>::new(),
                )])
                .with_field_effects([unpowered_idles()]),
        );
        registry.register(building("warpgate", 2, 4).with_field_effects([unpowered_idles()]));
        // A library researches lore, a battery fights through a turret, and an
        // acolyte runs twice as fast inside its own player's power.
        let lore = registry.register_research(
            "lore",
            ResearchDef::new(
                costs::cost(Vec::<(String, u32)>::new()),
                10,
                None,
                Vec::<String>::new(),
            ),
        );
        registry.register(
            building("library", 2, 4)
                .with_researcher([lore])
                .with_field_effects([unpowered_idles()]),
        );
        let battery_gun = registry.register_turret(
            "battery_gun",
            TurretDef::new(
                Weapon::new(utils::GROUND, Delivery::Instant, None),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            building("battery", 1, 2)
                .with_sight_range(8)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(360))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(6))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(6))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(2))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::ONE)
                .with_turrets([TurretMount::new(
                    battery_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )])
                .with_field_effects([unpowered_idles()]),
        );
        registry.register(mover("acolyte").with_field_effects([FieldEffect::new(
            power,
            FieldAffiliation::Own,
            FieldSide::Inside,
            FieldEffectKind::Modifiers(vec![modifier(
                EntityStatId::SPEED,
                ModifierOp::PercentAdd,
                "1.0",
            )]),
        )]));
        registry.register(
            building("cannon", 1, 20)
                .with_attack(utils::weapon(utils::GROUND), 10, 6, 6, 2, 1)
                .with_sight_range(8)
                .with_field_effects([unpowered_idles()]),
        );
        registry.register(
            building("hangar", 2, 2)
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ONE)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ONE)
                .with_transporter(
                    ["zealot"],
                    BoardingPolicy::Own,
                    PassengerFate::Eject,
                    PassengerConduct::Shelter,
                )
                .with_field_effects([unpowered_idles()]),
        );
        // A crystal yields to nobody while no power covers it.
        registry.register(
            EntityTypeDef::new("crystal")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("crystal", DepletionPolicy::Persist)
                .with_field_effects([FieldEffect::new(
                    power,
                    FieldAffiliation::Anyone,
                    FieldSide::Outside,
                    FieldEffectKind::Disabled,
                )]),
        );
        registry.register(
            mover("probe")
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(3))
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_builder(
                    ["pylon", "gateway", "cannon"],
                    BuilderAttendance::Crew(WorkPresence::Present),
                )
                .with_repairer(
                    ["structure"],
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present,
                    false,
                    RepairCost::Free,
                    None,
                )
                .with_resource_carrier([("crystal", HarvestData::new(5, 2, WorkPresence::Present))])
                .with_field_effects([unpowered_idles()]),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(100)
                .with_dying(1, None),
        );

        // Casts: an overlord spews a creep patch on a cell; a scourer clears one.
        let spew = registry.register_skill(
            "spew",
            SkillDef {
                cooldown: 1,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Position,
                    effect: EntityCastEffect::Field {
                        field: creep,
                        radius: 1,
                        action: FieldAction::Cover,
                    },
                },
                requires: Vec::new(),
            },
        );
        let scour = registry.register_skill(
            "scour",
            SkillDef {
                cooldown: 1,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Position,
                    effect: EntityCastEffect::Field {
                        field: creep,
                        radius: 2,
                        action: FieldAction::Clear,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(mover("overlord").with_skills([spew, scour]));

        registry.register(
            mover("worker")
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(3))
                .with_builder(
                    ["spore", "bunker", "gateway", "pylon", "hive", "nest"],
                    BuilderAttendance::Crew(WorkPresence::Present),
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// What `player` currently knows of the cell, on its own, without allies.
fn seen_by(app: &App, player: PlayerId, x: u32, y: u32) -> CellVisibility {
    app.world().resource::<VisibilityGrid>().get(player, x, y)
}

/// Ticks the researcher has put into its research.
fn research_progress(app: &App, researcher: Entity) -> u32 {
    app.world()
        .get::<ResearchComponent>(researcher)
        .expect("a working researcher carries its progress")
        .progress
}

/// Ticks the trainer has put into the front entry of its queue.
fn training_progress(app: &App, trainer: Entity) -> u32 {
    app.world()
        .get::<TrainComponent>(trainer)
        .expect("a working trainer carries its progress")
        .progress
}

/// Whether `player` or any ally currently sees the cell.
fn team_sees(app: &App, player: PlayerId, x: u32, y: u32) -> bool {
    let world = app.world();
    world
        .resource::<VisibilityGrid>()
        .is_visible_to(world.resource::<GameSession>(), player, x, y)
}

/// Whether anyone covers the cell in `field`.
fn covered_by_anyone(app: &App, field: FieldId, x: u32, y: u32) -> bool {
    !app.world()
        .resource::<FieldGrid>()
        .covered(field, CellPos::new(x, y))
        .is_empty()
}

/// Whether the field rules let `player` place `type_name` with its anchor on the cell.
fn allows(app: &App, player: PlayerId, type_name: &str, x: u32, y: u32) -> bool {
    let world = app.world();
    let def = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .unwrap();
    fields::allows_placement(world, Some(player), def, CellPos::new(x, y))
}

/// Orders `builder` to raise `type_name` with its anchor on the cell.
fn build(
    app: &mut App,
    worker: ferrets_simulation::simulation_id::SimulationId,
    type_name: &str,
    x: u32,
    y: u32,
) {
    utils::push_command(
        app,
        PlayerCommand::BuildEntity {
            builder: worker,
            type_name: type_name.into(),
            position: utils::pos(x, y),
            flush: true,
        },
    );
}
