//! The selection HUD: what the panel reports and withholds about the one entity
//! picked, and the command card rebuilding to one button of each kind.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::PendingInput;
use ferrets_content::{
    coverage::Coverage,
    entity_buffs::{EntityBuffDef, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry,
    },
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    skills::{Casting, EntityCastEffect, Recipient, SkillCaster, SkillDef},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
    terrains::Terrains,
};
use ferrets_demo::{
    hud::{self, SelectionText, SupplyText},
    input::{self, AimHover, AimVerdict, InputMode, Inspected, Leading, TargetedOrder},
    render::{ObserverPerspective, Sighted},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_nav_grid::{layer_mask::LayerMask, terrain_mask::TerrainMask};
use ferrets_simulation::{
    beside::Lean,
    command::{PlayerCommand, SelectMode, SkillTarget},
    components::{
        entity_skills::SkillsComponent,
        field_source::FieldReach,
        hidden::Inside,
        last_hit,
        order_queue::OrderQueueComponent,
        rise::{self, RiseAttendance, RiseOrigin, RisingComponent},
        train::TrainQueueComponent,
        warp::WarpComponent,
    },
    entity_index::EntityIndex,
    game_loop::{self, buffs::Bearing},
    movement_model::MovementModel,
    order::Order,
    player_research::PlayerResearch,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, local_role::LocalRole},
    simulation_id::SimulationId,
    spawn, supply,
    visibility::Sighting,
};

//
// ─── Selection panel ────────────────────────────────────────────────────────
//

#[test]
fn panel_names_own_pick_and_reports_its_health() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, grunt) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let shown = panel_text(&mut app);
    assert!(shown.contains("Grunt"), "the pick is named: {shown:?}");
    assert!(
        shown.contains("Health: "),
        "a pick in sight reports its health: {shown:?}"
    );
}

#[test]
fn panel_names_ground_under_pick() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, grunt) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let shown = panel_text(&mut app);
    assert_eq!(
        panel_field(&shown, "Terrain"),
        Some("Terrain: Land"),
        "{shown:?}"
    );

    // The sea's top rows are deep water.
    let (_, naga) = utils::create_entity(app.world_mut(), "naga", utils::cell(5, 5), Some(0))
        .expect("the demo content defines a naga");
    utils::select(&mut app, naga, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let shown = panel_text(&mut app);
    assert_eq!(
        panel_field(&shown, "Terrain"),
        Some("Terrain: Deep Water"),
        "{shown:?}"
    );

    // A 2×2 stilt walker over row 11, the sea's last shallow row, and row
    // 12, the land below it: both terrains, in name order.
    let (_, walker) =
        utils::create_entity(app.world_mut(), "stilt_walker", utils::cell(5, 11), Some(0))
            .expect("the demo content defines a stilt walker");
    utils::select(&mut app, walker, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let shown = panel_text(&mut app);
    assert_eq!(
        panel_field(&shown, "Terrain"),
        Some("Terrain: Land, Shallow Water"),
        "{shown:?}"
    );
}

#[test]
fn panel_lines_label_each_field() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, grunt) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The name, then its vitals, then where it stands and how: each field
    // labeled, fields on a line set apart by a bar.
    assert_eq!(
        panel_text(&mut app),
        "Grunt #1\nHealth: 60/60 | Attack: 10 | Speed: 0.30\nTerrain: Land | Stance: Defend"
    );
}

#[test]
fn panel_reports_energy_against_its_maximum() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, archer) = utils::create_entity(app.world_mut(), "archer", utils::cell(20, 20), Some(0))
        .expect("the demo content defines an archer");
    utils::select(&mut app, archer, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // An archer trains at a quarter of its 60: 15, and four ticks of 0.1
    // regeneration still read 15.
    let shown = panel_text(&mut app);
    assert!(
        shown.contains("Energy: 15/60"),
        "the energy reads against its maximum: {shown:?}"
    );
}

#[test]
fn panel_names_rising_entity_by_what_brought_it_in() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (farm, farm_id) =
        utils::create_rising(app.world_mut(), "farm", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a farm");
    let warp_gate = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("warp_gate")
        .expect("the demo content defines a warp gate");
    let raise_burrow = app
        .world()
        .resource::<ContentRegistry>()
        .skill("raise_burrow")
        .expect("the demo content defines raise_burrow");
    utils::select(&mut app, farm_id, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // The panel read straight after each origin is set, with no tick between
    // to judge the farm by it.
    for (origin, words) in [
        (
            RiseOrigin::Warped {
                warper_type: warp_gate,
                payer: 0,
                ready_at: 0,
            },
            "Warping in",
        ),
        (
            RiseOrigin::Summoned {
                skill: raise_burrow,
            },
            "Summoning",
        ),
    ] {
        app.world_mut()
            .get_mut::<RisingComponent>(farm)
            .expect("a rising entity carries its rising state")
            .origin = origin;

        let shown = panel_text(&mut app);
        assert!(shown.contains(words), "{origin:?} reads {words}: {shown:?}");
    }
}

#[test]
fn panel_names_rising_entity_and_buff_outage_as_simulation_judges_them() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (farm, farm_id) =
        utils::create_rising(app.world_mut(), "farm", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a farm");
    utils::select(&mut app, farm_id, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let shown = panel_text(&mut app);
    assert_eq!(
        panel_field(&shown, "Status"),
        Some("Status: Under construction"),
        "a rising entity reads under construction: {shown:?}"
    );

    rise::mark_as_risen(app.world_mut(), farm);
    let stunned = {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_entity_buff(
            "stunned",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        )
    };
    assert_eq!(
        game_loop::buffs::apply_entity_buff(app.world_mut(), farm, stunned),
        Bearing::Borne
    );
    let shown = panel_text(&mut app);
    assert_eq!(
        panel_field(&shown, "Status"),
        Some("Status: Disabled"),
        "a buff outage reads disabled: {shown:?}"
    );
    assert_eq!(
        panel_field(&shown, "Buffs"),
        Some("Buffs: Stunned"),
        "{shown:?}"
    );
}

#[test]
fn supply_readout_rounds_and_turns_red_on_what_it_shows() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    app.world_mut()
        .spawn((SupplyText, Text::new(""), TextColor::default()));
    utils::create_entity(app.world_mut(), "farm", utils::cell(24, 24), Some(0))
        .expect("the demo content defines a farm");
    let (grunt, _) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    let laden = {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_entity_buff(
            "laden",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    EntityModifier {
                        stat: EntityStatId::SUPPLY_COST,
                        op: ModifierOp::FlatAdd,
                        magnitude: "4.5".parse().expect("4.5 is a value"),
                    },
                ]))],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        )
    };
    assert_eq!(
        game_loop::buffs::apply_entity_buff(app.world_mut(), grunt, laden),
        Bearing::Borne
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(6));
    assert_eq!(supply::used(app.world(), 0), utils::fixed("5.5"));

    // A grunt's 1 + 4.5 = 5.5 used, read as 6, against the farm's 6: red on
    // what is shown, though 5.5 of 6 would still admit a half.
    app.world_mut()
        .run_system_once(hud::update_supply)
        .expect("the supply system runs");
    let mut query = app.world_mut().query::<(&Text, &TextColor, &SupplyText)>();
    let (text, color, _) = query.single(app.world()).expect("one readout");
    assert_eq!(text.0, "Supply: 6/6");
    assert_eq!(color.0, Color::srgb(1.0, 0.35, 0.3));
}

#[test]
fn panel_names_pick_simulation_holds_disabled() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    // A gateway with no power of its own under it stands switched off.
    let (_, gateway) =
        utils::create_entity(app.world_mut(), "gateway", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a gateway");
    utils::select(&mut app, gateway, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let shown = panel_text(&mut app);
    assert!(
        shown.contains("Disabled"),
        "an unpowered gateway reads disabled: {shown:?}"
    );
}

#[test]
fn panel_withholds_live_values_once_rival_pick_leaves_sight() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    // A rival's unit, lit by a scout of the side's own: what the side does not
    // own, it reads only while it makes it out.
    utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(22, 20), Some(1))
            .expect("the demo content defines a grunt");
    utils::run_ticks(&mut app, 2);
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // The stamp the sighting pass would write for a lit rival, by hand: the
    // panel reads the stamp, and an entity never stamped reads as unseen.
    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Seen));
    assert!(panel_text(&mut app).contains("Health: "));

    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Unseen));

    let shown = panel_text(&mut app);
    assert!(
        shown.contains("Out of sight"),
        "the panel says the side has lost it: {shown:?}"
    );
    assert!(
        !shown.contains("Health: "),
        "health is not read off an entity the side cannot see: {shown:?}"
    );
    assert!(
        !shown.contains("Speed"),
        "nor is anything else live: {shown:?}"
    );
}

#[test]
fn glimpsed_rival_pick_withholds_live_values_too() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(22, 20), Some(1))
            .expect("the demo content defines a grunt");
    utils::run_ticks(&mut app, 2);
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // A glimpse places an entity without making it out, so it reads no better
    // than an unseen one.
    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Glimpsed));

    let shown = panel_text(&mut app);
    assert!(shown.contains("Out of sight"), "{shown:?}");
    assert!(!shown.contains("Health: "), "{shown:?}");
}

#[test]
fn panel_reports_own_pick_that_carries_no_sighting() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // A worker down a mine or inside the site it raises is off the map, and
    // the sighting rule calls that unseen for its own side too. Its owner
    // still knows what it has.
    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Unseen));

    let shown = panel_text(&mut app);
    assert!(
        shown.contains("Health: "),
        "a side reads its own wherever they are: {shown:?}"
    );
    assert!(
        !shown.contains("Out of sight"),
        "and is never told it has lost them: {shown:?}"
    );
}

#[test]
fn panel_keeps_own_pick_that_goes_off_map() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(panel_text(&mut app).contains("Health: "));

    // Into a mine or aboard a wagon, the pick stays a pick: its side reads it
    // where it is.
    spawn::hide_entity(app.world_mut(), entity, Inside::Nothing);
    let shown = panel_text(&mut app);
    assert!(
        shown.contains("Health: "),
        "a side reads its own off the map too: {shown:?}"
    );
}

//
// ─── Command card ───────────────────────────────────────────────────────────
//

#[test]
fn rebuilding_command_card_leaves_one_button_of_each_kind() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    app.world_mut().init_resource::<Inspected>();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    // A barracks trains and a blacksmith researches, so one card carries both
    // of the cancel buttons the rebuild has to clear.
    let (_, barracks) =
        utils::create_entity(app.world_mut(), "barracks", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a barracks");
    app.world_mut().insert_resource(Leading(Some(barracks)));

    // Three builds of the same card: the first raises the buttons, and each
    // rebuild must clear what the last one left behind.
    for _ in 0..3 {
        app.world_mut().resource_mut::<Leading>().set_changed();
        app.world_mut()
            .run_system_once(hud::update_command_card)
            .expect("the command card system runs");
    }

    assert_eq!(
        count::<hud::CancelTrainButton>(&mut app),
        1,
        "one cancel-training button stands, whatever the card rebuilt"
    );
    assert_eq!(
        count::<hud::TrainButton>(&mut app),
        1,
        "and the train button it sits beside is not doubled either"
    );

    // The same for a card whose building researches rather than trains.
    let (_, blacksmith) =
        utils::create_entity(app.world_mut(), "blacksmith", utils::cell(26, 26), Some(0))
            .expect("the demo content defines a blacksmith");
    for _ in 0..3 {
        app.world_mut().insert_resource(Leading(Some(blacksmith)));
        app.world_mut()
            .run_system_once(hud::update_command_card)
            .expect("the command card system runs");
    }
    assert_eq!(
        count::<hud::CancelResearchButton>(&mut app),
        1,
        "one cancel-research button stands, whatever the card rebuilt"
    );
    assert_eq!(
        count::<hud::CancelTrainButton>(&mut app),
        0,
        "and the trainer's button went with the card that carried it"
    );
}

#[test]
fn pressing_cancel_unit_sends_one_command_for_leading_trainer() {
    let mut app = card_app();
    let (_, barracks) =
        utils::create_entity(app.world_mut(), "barracks", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a barracks");
    app.world_mut().insert_resource(Leading(Some(barracks)));
    app.world_mut()
        .spawn((hud::CancelTrainButton, Interaction::Pressed));

    app.world_mut()
        .run_system_once(hud::cancel_train_card_input)
        .expect("the input system runs");

    assert_eq!(
        app.world().resource::<PendingInput>().queued(),
        [PlayerCommand::CancelTrain {
            trainer: barracks,
            slot: 0,
        }],
        "slot 0 is the unit in progress"
    );
}

#[test]
fn pressing_cancel_research_on_idle_researcher_sends_nothing() {
    let mut app = card_app();
    let (_, blacksmith) =
        utils::create_entity(app.world_mut(), "blacksmith", utils::cell(24, 24), Some(0))
            .expect("the demo content defines a blacksmith");
    app.world_mut().insert_resource(Leading(Some(blacksmith)));
    app.world_mut()
        .spawn((hud::CancelResearchButton, Interaction::Pressed));

    app.world_mut()
        .run_system_once(hud::cancel_research_card_input)
        .expect("the input system runs");

    assert!(
        app.world().resource::<PendingInput>().queued().is_empty(),
        "a researcher working nothing has no topic to name"
    );
}

#[test]
fn pressing_cancel_research_names_topic_from_researcher_own_queue() {
    let mut app = card_app();
    let (entity, blacksmith) =
        utils::create_entity(app.world_mut(), "blacksmith", utils::cell(24, 24), Some(0))
            .expect("the demo content defines a blacksmith");
    let iron = app
        .world()
        .resource::<ContentRegistry>()
        .research("iron_weapons")
        .expect("the demo content defines iron weapons");
    app.world_mut()
        .get_mut::<OrderQueueComponent>(entity)
        .expect("simulation entities carry an order queue")
        .push(Order::Research { research: iron }, None);
    app.world_mut().insert_resource(Leading(Some(blacksmith)));
    app.world_mut()
        .spawn((hud::CancelResearchButton, Interaction::Pressed));

    app.world_mut()
        .run_system_once(hud::cancel_research_card_input)
        .expect("the input system runs");

    assert_eq!(
        app.world().resource::<PendingInput>().queued(),
        [PlayerCommand::CancelResearch {
            researcher: blacksmith,
            research: iron,
        }],
    );
}

#[test]
fn rising_card_offers_cancel_unless_it_is_summon() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    let (farm, farm_id) =
        utils::create_rising(app.world_mut(), "farm", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a farm");
    let raise_burrow = app
        .world()
        .resource::<ContentRegistry>()
        .skill("raise_burrow")
        .expect("the demo content defines raise_burrow");
    app.world_mut().insert_resource(Leading(Some(farm_id)));

    let warp_gate = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("warp_gate")
        .expect("the demo content defines a warp gate");
    // A founded site or a unit warping in can be called off; a summon is a
    // cast that has landed.
    for (origin, buttons) in [
        (RiseOrigin::Founded { payer: Some(0) }, 1),
        (
            RiseOrigin::Warped {
                warper_type: warp_gate,
                payer: 0,
                ready_at: 0,
            },
            1,
        ),
        (
            RiseOrigin::Summoned {
                skill: raise_burrow,
            },
            0,
        ),
    ] {
        app.world_mut()
            .get_mut::<RisingComponent>(farm)
            .expect("a rising entity carries its rising state")
            .origin = origin;
        app.world_mut().resource_mut::<Leading>().set_changed();
        app.world_mut()
            .run_system_once(hud::update_command_card)
            .expect("the command card system runs");
        assert_eq!(
            count::<hud::CancelBuildButton>(&mut app),
            buttons,
            "{origin:?}"
        );
    }
}

#[test]
fn cancel_buttons_gray_out_with_nothing_to_call_off() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));

    // A barracks with nothing queued: the cancel-unit button is raised with
    // the card and grayed by the availability pass, since there is nothing
    // to call off; one entry queued brings it back.
    let (entity, barracks) =
        utils::create_entity(app.world_mut(), "barracks", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a barracks");
    app.world_mut().insert_resource(Leading(Some(barracks)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    assert_eq!(
        card_color::<hud::CancelTrainButton>(&mut app),
        hud::BUTTON_NORMAL
    );

    recolor_card(&mut app);
    assert_eq!(
        card_color::<hud::CancelTrainButton>(&mut app),
        hud::CARD_DISABLED,
        "an empty train queue is nothing to call off"
    );

    app.world_mut()
        .get_mut::<TrainQueueComponent>(entity)
        .expect("a trainer carries a train queue")
        .0
        .push_back("grunt".to_string());
    recolor_card(&mut app);
    assert_eq!(
        card_color::<hud::CancelTrainButton>(&mut app),
        hud::BUTTON_NORMAL,
        "one entry queued is one to call off"
    );

    // The same for a researcher, whose topic sits in its order queue.
    let (entity, blacksmith) =
        utils::create_entity(app.world_mut(), "blacksmith", utils::cell(26, 26), Some(0))
            .expect("the demo content defines a blacksmith");
    app.world_mut().insert_resource(Leading(Some(blacksmith)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    assert_eq!(
        card_color::<hud::CancelResearchButton>(&mut app),
        hud::BUTTON_NORMAL
    );

    recolor_card(&mut app);
    assert_eq!(
        card_color::<hud::CancelResearchButton>(&mut app),
        hud::CARD_DISABLED,
        "a researcher working nothing has nothing to call off"
    );

    let iron = app
        .world()
        .resource::<ContentRegistry>()
        .research("iron_weapons")
        .expect("the demo content defines iron weapons");
    app.world_mut()
        .get_mut::<OrderQueueComponent>(entity)
        .expect("simulation entities carry an order queue")
        .push(Order::Research { research: iron }, None);
    recolor_card(&mut app);
    assert_eq!(
        card_color::<hud::CancelResearchButton>(&mut app),
        hud::BUTTON_NORMAL,
        "a topic under way is one to call off"
    );
}

#[test]
fn hovered_card_button_names_its_price_or_what_it_lacks() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));

    // A factory with no tech lab docked and no siege tech: the tank's button
    // is grayed and says what either branch would take; the wraith's is live
    // and says what it costs — 150 gold, 100 wood and 90 ticks, 4.5 Seconds.
    let (_, factory) =
        utils::create_entity(app.world_mut(), "factory", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a factory");
    app.world_mut().insert_resource(Leading(Some(factory)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    let buttons: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<hud::TrainButton>>()
        .iter(app.world())
        .collect();
    assert_eq!(buttons.len(), 2, "a tank and a wraith");
    assert_eq!(
        hover_each::<hud::TrainButton>(&mut app),
        vec![
            "150 Gold, 100 Wood, 4.5 Seconds".to_string(),
            "Needs (a Tech Lab docked or Siege Tech)".to_string(),
        ]
    );

    // Nothing hovered, nothing said.
    for button in &buttons {
        *app.world_mut().get_mut::<Interaction>(*button).unwrap() = Interaction::None;
    }
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "");
}

#[test]
fn hovered_skill_names_its_cost_or_why_it_cannot_cast() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (archer, archer_id) =
        utils::create_entity(app.world_mut(), "archer", utils::cell(20, 20), Some(0))
            .expect("the demo content defines an archer");
    app.world_mut().insert_resource(Leading(Some(archer_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    // Battle focus: 30 energy, and 80 ticks of cooldown, 4 Seconds.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "30 Energy, 4 Seconds Cooldown");

    let battle_focus = app
        .world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("the demo content defines battle focus");
    app.world_mut()
        .get_mut::<SkillsComponent>(archer)
        .expect("an archer carries its skills")
        .start_cooldown(battle_focus, 80);
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Cooling down");

    app.world_mut()
        .get_mut::<SkillsComponent>(archer)
        .unwrap()
        .reset_cooldown(battle_focus);
    // Led by a rising entity: whatever the skill, it cannot cast yet.
    let (_, farm_id) = utils::create_rising(app.world_mut(), "farm", utils::cell(28, 20), Some(0))
        .expect("the demo content defines a farm");
    app.world_mut().insert_resource(Leading(Some(farm_id)));
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Still under construction");

    app.world_mut().insert_resource(Leading(Some(archer_id)));
    let stunned = {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_entity_buff(
            "stunned",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        )
    };
    assert_eq!(
        game_loop::buffs::apply_entity_buff(app.world_mut(), archer, stunned),
        Bearing::Borne
    );
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Switched off");
    // Switched off while cooling down: the caster's state is named first.
    app.world_mut()
        .get_mut::<SkillsComponent>(archer)
        .unwrap()
        .start_cooldown(battle_focus, 80);
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Switched off");

    // Led by something that knows no battle focus, or by nothing at all.
    let (_, grunt_id) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(24, 20), Some(0))
            .expect("the demo content defines a grunt");
    app.world_mut().insert_resource(Leading(Some(grunt_id)));
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "It cannot cast this");
    app.world_mut().insert_resource(Leading(None));
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Nothing selected can cast it");
}

#[test]
fn hovered_skill_names_ground_it_needs() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, naga_id) = utils::create_entity(app.world_mut(), "naga", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a naga");
    app.world_mut().insert_resource(Leading(Some(naga_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Needs standing on Deep Water (Ground)");
}

#[test]
fn armed_skill_names_why_it_refuses_hovered_target() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, shaman_id) =
        utils::create_entity(app.world_mut(), "shaman", utils::cell(30, 30), Some(0))
            .expect("the demo content defines a shaman");
    let (_, barracks_id) =
        utils::create_entity(app.world_mut(), "barracks", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a barracks");
    let second_wind = app
        .world()
        .resource::<ContentRegistry>()
        .skill("second_wind")
        .expect("the demo content defines second wind");
    app.world_mut().insert_resource(Leading(Some(shaman_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    app.world_mut()
        .insert_resource(InputMode::Targeting(TargetedOrder::Skill(second_wind)));

    // Second wind mends only the biological: a barracks is not one.
    app.world_mut()
        .insert_resource(AimHover(Some(SkillTarget::Entity(barracks_id))));
    app.world_mut()
        .run_system_once(input::judge_aim)
        .expect("the aim judge runs");
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Not something it can be cast on");

    // Over nothing, the line has no refusal to name.
    app.world_mut().insert_resource(AimHover(None));
    app.world_mut()
        .run_system_once(input::judge_aim)
        .expect("the aim judge runs");
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "");
}

#[test]
fn hovered_aim_past_transient_reach_reads_out_of_reach() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, huntress_id) =
        utils::create_entity(app.world_mut(), "huntress", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a huntress");
    // A second huntress sees round the far cell, so the landing is sighted.
    utils::create_entity(app.world_mut(), "huntress", utils::cell(32, 20), Some(0))
        .expect("the demo content defines a huntress");
    utils::run_ticks(&mut app, 1);
    let dash = app
        .world()
        .resource::<ContentRegistry>()
        .skill("dash")
        .expect("the demo content defines dash");
    app.world_mut().insert_resource(Leading(Some(huntress_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    app.world_mut()
        .insert_resource(InputMode::Targeting(TargetedOrder::Skill(dash)));

    // Ten cells off, past its four: cast transient, it never walks there.
    app.world_mut()
        .insert_resource(AimHover(Some(SkillTarget::Cell(utils::cell(30, 20)))));
    app.world_mut()
        .run_system_once(input::judge_aim)
        .expect("the aim judge runs");
    assert_eq!(
        *app.world().resource::<AimVerdict>(),
        AimVerdict::OutOfReach
    );
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Out of reach");

    // Three cells off, within it.
    app.world_mut()
        .insert_resource(AimHover(Some(SkillTarget::Cell(utils::cell(23, 20)))));
    app.world_mut()
        .run_system_once(input::judge_aim)
        .expect("the aim judge runs");
    assert_eq!(*app.world().resource::<AimVerdict>(), AimVerdict::Takes);
}

#[test]
fn armed_skill_stays_armed_while_its_leader_can_cast_it() {
    let mut app = card_app();
    let (archer, archer_id) =
        utils::create_entity(app.world_mut(), "archer", utils::cell(20, 20), Some(0))
            .expect("the demo content defines an archer");
    let (_, other_id) =
        utils::create_entity(app.world_mut(), "archer", utils::cell(24, 20), Some(0))
            .expect("the demo content defines an archer");
    let (_, grunt_id) =
        utils::create_entity(app.world_mut(), "grunt", utils::cell(28, 20), Some(0))
            .expect("the demo content defines a grunt");
    let battle_focus = app
        .world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("the demo content defines battle focus");
    let armed = || InputMode::Targeting(TargetedOrder::Skill(battle_focus));
    let is_armed = |app: &App| {
        matches!(
            *app.world().resource::<InputMode>(),
            InputMode::Targeting(TargetedOrder::Skill(skill)) if skill == battle_focus
        )
    };
    let disarm = |app: &mut App| {
        app.world_mut()
            .run_system_once(input::disarm_lost_caster)
            .expect("the disarm system runs");
    };
    app.world_mut().insert_resource(armed());

    // Led by an archer, then by another: both cast it, so it stays armed.
    app.world_mut().insert_resource(Leading(Some(archer_id)));
    disarm(&mut app);
    assert!(is_armed(&app));
    app.world_mut().insert_resource(Leading(Some(other_id)));
    disarm(&mut app);
    assert!(is_armed(&app));

    // Led by a grunt, whose type carries no battle focus: disarmed.
    app.world_mut().insert_resource(Leading(Some(grunt_id)));
    disarm(&mut app);
    assert!(!is_armed(&app));

    // Armed again, its archer taken by player 1: disarmed.
    app.world_mut().insert_resource(armed());
    app.world_mut().insert_resource(Leading(Some(archer_id)));
    spawn::change_owner(app.world_mut(), archer, 1, SimulationId(999), Lean::Nothing);
    disarm(&mut app);
    assert!(!is_armed(&app));
}

#[test]
fn hovered_skill_names_health_it_costs() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let frenzy_ritual = app
        .world()
        .resource::<ContentRegistry>()
        .research("frenzy_ritual")
        .expect("the demo content defines the frenzy ritual");
    app.world_mut()
        .resource_mut::<PlayerResearch>()
        .mark_completed(0, frenzy_ritual);
    let (_, grunt) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    app.world_mut().insert_resource(Leading(Some(grunt)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    // Blood rite: 10 gold and 8 health, and 160 ticks of cooldown, 8 Seconds.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "10 Gold, 8 Health, 8 Seconds Cooldown");
}

#[test]
fn cooldowns_leave_last_selections_skill_buttons_alone_until_card_rebuilds() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    let (_, grunt) = utils::create_entity(app.world_mut(), "grunt", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    app.world_mut().insert_resource(Leading(Some(grunt)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    let before = skill_button_labels(&mut app);

    // The selection moves to a caster without the grunt's blood rite before
    // the card is rebuilt for it.
    let (_, necromancer) =
        utils::create_entity(app.world_mut(), "necromancer", utils::cell(24, 20), Some(0))
            .expect("the demo content defines a necromancer");
    app.world_mut().insert_resource(Leading(Some(necromancer)));
    app.world_mut()
        .run_system_once(hud::update_skill_cooldowns)
        .expect("the cooldown labels update");

    assert_eq!(skill_button_labels(&mut app), before);
}

#[test]
fn captured_leading_entity_loses_its_skill_buttons() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    // One system kept across runs, so it rebuilds only on what changed.
    let card = app.world_mut().register_system(hud::update_command_card);
    let (archer, archer_id) =
        utils::create_entity(app.world_mut(), "archer", utils::cell(20, 20), Some(0))
            .expect("the demo content defines an archer");
    app.world_mut().insert_resource(Leading(Some(archer_id)));
    for _ in 0..2 {
        app.world_mut()
            .run_system(card)
            .expect("the command card system runs");
    }
    assert_eq!(skill_button_labels(&mut app).len(), 1, "battle focus");

    // Taken by player 1 while still leading the selection: its card is
    // rebuilt as another side's, with no skills to offer.
    spawn::change_owner(app.world_mut(), archer, 1, SimulationId(999), Lean::Nothing);
    app.world_mut()
        .run_system(card)
        .expect("the command card system runs");
    assert!(skill_button_labels(&mut app).is_empty());
}

#[test]
fn hovered_player_skill_names_its_price_and_cooldown() {
    let mut app = card_app();
    app.world_mut().insert_resource(Leading(None));
    app.world_mut()
        .run_system_once(hud::setup_hud)
        .expect("the HUD sets up");
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 50);
    hover_only::<hud::PlayerSkillButton>(&mut app);

    // War drums: 50 gold, and 300 ticks of cooldown, 15 Seconds.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "50 Gold, 15 Seconds Cooldown");
}

#[test]
fn hovered_build_buttons_name_their_price_and_time() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 1000);
    let (_, drone) = utils::create_entity(app.world_mut(), "drone", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a drone");
    app.world_mut().insert_resource(Leading(Some(drone)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    // A tunnel is 100 gold and 50 wood in 80 ticks, 4 Seconds; a warren 150 gold
    // and 150 wood in 100, 5 Seconds; a spawning pit 200 gold and 100 wood in 120,
    // 6 Seconds; a tumor 25 gold in 40, 2 Seconds; a hatchery 400 gold in 200, 10 Seconds.
    assert_eq!(
        hover_each::<hud::BuildButton>(&mut app),
        vec![
            "100 Gold, 50 Wood, 4 Seconds".to_string(),
            "150 Gold, 150 Wood, 5 Seconds".to_string(),
            "200 Gold, 100 Wood, 6 Seconds".to_string(),
            "25 Gold, 2 Seconds".to_string(),
            "400 Gold, 10 Seconds".to_string(),
        ]
    );
}

#[test]
fn hovered_morph_names_its_price() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 100);
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "wood", 100);
    let (_, tower) =
        utils::create_entity(app.world_mut(), "watch_tower", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a watch tower");
    utils::select(&mut app, tower, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(tower)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::MorphButton>(&mut app);

    // Into a guard tower: 80 gold and 20 wood, and 60 ticks at 20 a second.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "80 Gold, 20 Wood, 3 Seconds");
}

#[test]
fn build_button_asks_nothing_of_ground_before_site_is_placed() {
    // On a field of land, a finished tunnel at (0, 0) never walks and stands
    // where a tunnel anchored at (0, 0) would rise: the card has no site yet,
    // so the button stands and its hint is the price — 100 gold, 50 wood, 80
    // ticks at 20 a second.
    let mut app = with_card_resources(utils::land_app_as(
        LocalRole::Player(0),
        MovementModel::Cell,
    ));
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, tunnel) = utils::create_entity(app.world_mut(), "tunnel", utils::cell(0, 0), Some(0))
        .expect("the demo content defines a tunnel");
    utils::select(&mut app, tunnel, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(tunnel)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::BuildButton>(&mut app);

    recolor_card(&mut app);

    assert_ne!(card_color::<hud::BuildButton>(&mut app), hud::CARD_DISABLED);
    assert_eq!(hint_text(&mut app), "100 Gold, 50 Wood, 4 Seconds");
}

#[test]
fn tunnel_under_construction_grays_its_build_button() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, drone) = utils::create_entity(app.world_mut(), "drone", utils::cell(10, 20), Some(0))
        .expect("the demo content defines a drone");
    let (_, tunnel) = spawn::spawn_founded(
        app.world_mut(),
        "tunnel",
        spawn::At::Cell(utils::cell(20, 20)),
        Some(0),
        FieldReach::Initial,
        drone,
        RiseAttendance::Unattended { founder: drone },
        &[],
    )
    .expect("the tunnel fits at (20, 20)");
    utils::select(&mut app, tunnel, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(tunnel)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::BuildButton>(&mut app);

    recolor_card(&mut app);

    assert_eq!(card_color::<hud::BuildButton>(&mut app), hud::CARD_DISABLED);
    assert_eq!(hint_text(&mut app), "Still under construction");
}

#[test]
fn bound_tunnel_grays_its_build_button_naming_its_other_end() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, tunnel) = utils::create_entity(app.world_mut(), "tunnel", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a tunnel");
    spawn::spawn_founded(
        app.world_mut(),
        "tunnel",
        spawn::At::Cell(utils::cell(30, 20)),
        Some(0),
        FieldReach::Initial,
        tunnel,
        RiseAttendance::Unattended { founder: tunnel },
        &[],
    )
    .expect("the other end fits at (30, 20)");
    utils::select(&mut app, tunnel, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(tunnel)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::BuildButton>(&mut app);

    recolor_card(&mut app);

    assert_eq!(card_color::<hud::BuildButton>(&mut app), hud::CARD_DISABLED);
    assert_eq!(hint_text(&mut app), "Already bound to its other end");
}

#[test]
fn pressing_grayed_unload_on_empty_burrow_sends_nothing() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    let (_, burrow) = utils::create_entity(app.world_mut(), "burrow", utils::cell(20, 20), Some(0))
        .expect("the demo content defines a burrow");
    utils::select(&mut app, burrow, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(burrow)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    recolor_card(&mut app);

    let mut unloads = app
        .world_mut()
        .query_filtered::<&mut Interaction, With<hud::UnloadButton>>();
    for mut interaction in unloads.iter_mut(app.world_mut()) {
        *interaction = Interaction::Pressed;
    }
    app.world_mut()
        .run_system_once(hud::unload_card_input)
        .expect("the input system runs");

    assert!(app.world().resource::<PendingInput>().queued().is_empty());
    assert!(matches!(
        app.world().resource::<InputMode>(),
        InputMode::Normal
    ));
}

#[test]
fn warp_button_stands_while_another_selected_gate_may_warp() {
    let mut app = card_app();
    let (cooling_id, ready_id) = warp_card(&mut app);

    // Led by a cooling gate alone, no warp button stands; with a ready gate of
    // its type selected beside it, the zealot's does.
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![cooling_id]);
    recolor_card(&mut app);
    let alone = open_warp_buttons(&mut app);
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![cooling_id, ready_id]);
    recolor_card(&mut app);
    let beside = open_warp_buttons(&mut app);
    assert_eq!((alone, beside), (0, 1));
}

#[test]
fn warp_hint_names_leaders_refusal_only_while_no_gate_may_warp() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (leader_id, ready_id) = warp_card(&mut app);
    let leader = app
        .world()
        .resource::<EntityIndex>()
        .alive(leader_id)
        .expect("the leading gate stands");

    // Led by a cooling gate alone: its refusal.
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![leader_id]);
    let alone = hover_each::<hud::WarpButton>(&mut app);
    // With a ready gate beside it: what the warp-in costs.
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![leader_id, ready_id]);
    let beside = hover_each::<hud::WarpButton>(&mut app);
    // Led by a ready gate whose own warp-in is on its way, alone.
    let tick = app.world().resource::<GameSession>().tick();
    app.world_mut()
        .get_mut::<WarpComponent>(leader)
        .expect("a warp gate keeps its cooldown")
        .ready_at = 0;
    app.world_mut()
        .resource_mut::<input::WarpsSent>()
        .0
        .push(input::SentWarp {
            warper: leader_id,
            tick,
            cooldown_end: 0,
        });
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![leader_id]);
    let sent = hover_each::<hud::WarpButton>(&mut app);
    // The zealot's and the stalker's, sorted; the stalker's 50 wood is more
    // than the player holds. The cooling leader's refusal for both; beside a
    // ready gate the zealot's price, 100 gold and 60 ticks, 3 Seconds, and the
    // leader's refusal for the stalker no gate may warp; alone again and
    // ready, the zealot's warp-in on its way and the stalker's price unmet.
    assert_eq!(
        (alone, beside, sent),
        (
            vec!["Cooling down".to_string(), "Cooling down".to_string()],
            vec![
                "100 Gold, 3 Seconds".to_string(),
                "Cooling down".to_string()
            ],
            vec![
                "Not enough resources".to_string(),
                "Warp-in on its way".to_string()
            ],
        )
    );
}

#[test]
fn pressing_warp_gates_warp_button_arms_warp_placement() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    let (_, gate) =
        utils::create_entity(app.world_mut(), "warp_gate", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a warp gate");
    utils::select(&mut app, gate, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(gate)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    // A warp gate trains nothing: its card lists warp buttons alone, the
    // zealot first.
    assert_eq!(count::<hud::TrainButton>(&mut app), 0);
    let mut buttons = app
        .world_mut()
        .query_filtered::<&mut Interaction, With<hud::WarpButton>>();
    let mut first = buttons
        .iter_mut(app.world_mut())
        .next()
        .expect("a warp gate's card lists what it warps in");
    *first = Interaction::Pressed;
    app.world_mut()
        .run_system_once(hud::warp_card_input)
        .expect("the input system runs");

    assert!(app.world().resource::<PendingInput>().queued().is_empty());
    let mode = app.world().resource::<InputMode>();
    assert!(matches!(mode, InputMode::PlacingWarp(unit) if unit == "zealot"));
}

#[test]
fn grayed_button_names_ground_and_layers_it_needs() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    register_diver(&mut app);
    let (_, diver_id) =
        utils::create_entity(app.world_mut(), "diver", utils::cell(20, 20), Some(0))
            .expect("the fixture registers a diver");
    app.world_mut().insert_resource(Leading(Some(diver_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::MorphButton>(&mut app);

    // On land on the ground layer: touching either water on any layer is
    // unmet, the terrains in name order and the layers left unsaid as they
    // are all of them; not standing anywhere on the map on the ground layer
    // is unmet too, its layer named.
    recolor_card(&mut app);
    assert_eq!(
        hint_text(&mut app),
        "Needs touching Deep Water or Shallow Water and not standing on the map (Ground)"
    );
}

#[test]
fn self_cast_its_buff_would_not_hold_on_grays_with_reason() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    register_diver(&mut app);
    let (_, dipper_id) =
        utils::create_entity(app.world_mut(), "dipper", utils::cell(20, 20), Some(0))
            .expect("the fixture registers a dipper");
    app.world_mut().insert_resource(Leading(Some(dipper_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    // `dip` asks nothing, but on land the buff it puts on its caster would
    // not hold: the button grays and says so.
    recolor_card(&mut app);
    assert_eq!(card_color::<hud::SkillButton>(&mut app), hud::CARD_DISABLED);
    assert_eq!(hint_text(&mut app), "The buff would not hold there");
}

#[test]
fn grayed_button_names_unmet_part_of_requirement_tree() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    register_totem(&mut app);
    let (totem, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::cell(20, 20), Some(0))
            .expect("the fixture registers a totem");
    // Hit this tick, and given an order it has not started.
    let tick = app.world().resource::<GameSession>().tick();
    last_hit::record(app.world_mut(), totem, totem_id, tick);
    app.world_mut()
        .get_mut::<OrderQueueComponent>(totem)
        .unwrap()
        .push(
            Order::Morph {
                type_name: "watch_tower".to_string(),
            },
            None,
        );
    app.world_mut().insert_resource(Leading(Some(totem_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::MorphButton>(&mut app);

    // An `any` with no branch met names each branch; one with a branch met
    // (health at least a tenth) says nothing; an `all` names each unmet
    // branch and not the met one (sight at least 1). Under 0.34 of the
    // health reads 34%; 40 ticks idle is 2 seconds, 200 unhurt 10 seconds.
    recolor_card(&mut app);
    assert_eq!(
        hint_text(&mut app),
        "Needs (a Spawning Pit or Frenzy Ritual) and health under 34% and energy at least 50 and Sight Range at least 99 and standing idle and 2 seconds idle and 10 seconds unhurt"
    );

    // Led by nothing, the button names the change it cannot start.
    app.world_mut().insert_resource(Leading(None));
    recolor_card(&mut app);
    assert_eq!(
        hint_text(&mut app),
        "Nothing selected can become a Watch Tower now"
    );
}

#[test]
fn hovered_researches_name_their_price_and_time() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, blacksmith) =
        utils::create_entity(app.world_mut(), "blacksmith", utils::cell(26, 26), Some(0))
            .expect("the demo content defines a blacksmith");
    app.world_mut().insert_resource(Leading(Some(blacksmith)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    // Iron weapons: 100 gold, 50 wood and 200 ticks, 10 Seconds; vitality drill:
    // 80 gold, 40 wood and 160 ticks, 8 Seconds.
    assert_eq!(
        hover_each::<hud::ResearchButton>(&mut app),
        vec![
            "100 Gold, 50 Wood, 10 Seconds".to_string(),
            "80 Gold, 40 Wood, 8 Seconds".to_string(),
        ]
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Spawns the panel node the selection system writes into, standing in for the
/// HUD setup a real game does on entering the scene.
fn spawn_panel(app: &mut App) {
    // A playing seat never reads either, but the system takes them anyway.
    app.world_mut().init_resource::<Inspected>();
    app.world_mut().init_resource::<ObserverPerspective>();
    app.world_mut().spawn((SelectionText, Text::new("")));
}

/// Runs the selection system and reads back what it wrote.
fn panel_text(app: &mut App) -> String {
    app.world_mut()
        .run_system_once(hud::update_selection)
        .expect("the selection system runs");
    let mut query = app.world_mut().query::<(&Text, &SelectionText)>();
    let (text, _) = query.single(app.world()).expect("one panel");
    text.0.clone()
}

/// How many entities carry the marker component.
fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&C>().iter(app.world()).count()
}

/// Runs the availability pass that recolors the card's gated buttons.
fn recolor_card(app: &mut App) {
    app.world_mut()
        .run_system_once(hud::update_card_availability)
        .expect("the availability system runs");
}

/// The background color of the one button carrying the marker component.
fn card_color<C: Component>(app: &mut App) -> Color {
    let mut query = app.world_mut().query::<(&BackgroundColor, &C)>();
    let (background, _) = query
        .single(app.world())
        .expect("one button of the kind on the card");
    background.0
}

/// A headless game with the resources the command-card input systems take.
fn card_app() -> App {
    with_card_resources(utils::demo_map_app(MovementModel::Cell))
}

/// `app` with the resources the command card systems read.
fn with_card_resources(mut app: App) -> App {
    app.world_mut().init_resource::<Inspected>();
    app.world_mut().init_resource::<ObserverPerspective>();
    app.world_mut().init_resource::<InputMode>();
    app.world_mut().init_resource::<AimHover>();
    app.world_mut().init_resource::<AimVerdict>();
    app
}

/// What the card's hint line reads.
fn hint_text(app: &mut App) -> String {
    let mut query = app.world_mut().query::<(&Text, &hud::CardHint)>();
    let (text, _) = query.single(app.world()).expect("one hint line");
    text.0.clone()
}

/// What the hint line reads with each button carrying the marker component
/// hovered in turn, sorted.
fn hover_each<C: Component>(app: &mut App) -> Vec<String> {
    let buttons: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<C>>()
        .iter(app.world())
        .collect();
    let mut hints: Vec<String> = Vec::new();
    for hovered in &buttons {
        for button in &buttons {
            *app.world_mut().get_mut::<Interaction>(*button).unwrap() = if button == hovered {
                Interaction::Hovered
            } else {
                Interaction::None
            };
        }
        recolor_card(app);
        hints.push(hint_text(app));
    }
    hints.sort();
    hints
}

/// Registers the `totem`: a ground piece with 100 health, 40 energy and sight
/// 5, whose one change of form, into a watch tower, requires any of a
/// spawning pit or the frenzy ritual; any of health at least a tenth or the
/// frenzy ritual; all of health under 0.34 of its maximum, energy at least 50,
/// sight at least 99 and sight at least 1; standing idle; 40 ticks idle; and
/// 200 ticks unhurt.
fn register_totem(app: &mut App) {
    let (land, ground) = {
        let registry = app.world().resource::<ContentRegistry>();
        (
            registry.domain("land").expect("the demo declares land"),
            registry.layer("ground").expect("the demo declares ground"),
        )
    };
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    let frenzy_ritual = registry
        .research("frenzy_ritual")
        .expect("the demo content defines the frenzy ritual");
    registry.register(
        EntityTypeDef::new("totem")
            .with_location(land, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(40))
            .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(5))
            .with_morphs([MorphTransition::new(
                "watch_tower",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::from(10),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                MorphReason::Change,
                Vec::new(),
                [
                    Requirement::Any(vec![
                        Requirement::EntityType("spawning_pit".to_string()),
                        Requirement::Research(frenzy_ritual),
                    ]),
                    Requirement::Any(vec![
                        Requirement::Health(Bound::Share(Threshold::AtLeast(utils::fixed("0.1")))),
                        Requirement::Research(frenzy_ritual),
                    ]),
                    Requirement::All(vec![
                        Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.34")))),
                        Requirement::Energy(Bound::Amount(Threshold::AtLeast(FixedU64::from_num(
                            50,
                        )))),
                        Requirement::Stat {
                            stat: EntityStatId::SIGHT_RANGE,
                            bound: Bound::Amount(Threshold::AtLeast(FixedU64::from_num(99))),
                        },
                        Requirement::Stat {
                            stat: EntityStatId::SIGHT_RANGE,
                            bound: Bound::Amount(Threshold::AtLeast(FixedU64::ONE)),
                        },
                    ]),
                    Requirement::Idle,
                    Requirement::IdleFor(40),
                    Requirement::UnhurtFor(200),
                ],
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )]),
    );
}

/// Registers the `diver`: a ground piece with 100 health whose one change of
/// form, into a watch tower, requires touching shallow or deep water on any
/// layer, and not standing anywhere on the map on the ground layer; and the
/// `dipper`, a ground piece with 100 health casting `dip`, which puts on
/// itself `soaked`, held while standing on shallow water on the ground layer,
/// and asks nothing. The registry is validated with them in.
fn register_diver(app: &mut App) {
    let (ground_domain, ground, air, shallow, deep) = {
        let registry = app.world().resource::<ContentRegistry>();
        (
            registry.domain("ground").expect("the demo declares ground"),
            registry.layer("ground").expect("the demo declares ground"),
            registry.layer("air").expect("the demo declares air"),
            registry
                .terrain("shallow_water")
                .expect("the demo declares shallow water"),
            registry
                .terrain("deep_water")
                .expect("the demo declares deep water"),
        )
    };
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    registry.register(
        EntityTypeDef::new("diver")
            .with_location(ground_domain, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(100))
            .with_morphs([MorphTransition::new(
                "watch_tower",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::from(10),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                MorphReason::Change,
                Vec::new(),
                [
                    Requirement::Over {
                        terrains: Terrains::Only(TerrainMask::from(shallow) | deep),
                        coverage: Coverage::Any,
                        layers: LayerMask::from(ground) | air,
                    },
                    Requirement::Unless(Box::new(Requirement::Over {
                        terrains: Terrains::Any,
                        coverage: Coverage::Every,
                        layers: ground.into(),
                    })),
                ],
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )]),
    );
    let soaked = registry.register_entity_buff(
        "soaked",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::Over {
                terrains: Terrains::Only(shallow.into()),
                coverage: Coverage::Every,
                layers: ground.into(),
            }),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    let dip = registry.register_skill(
        "dip",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::ApplyBuff {
                    buff: soaked,
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("dipper")
            .with_location(ground_domain, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(100))
            .with_skills([dip]),
    );
    registry.validate();
}

/// The field of the panel's text labeled `label`: the part of a line between
/// its ` | ` separators that starts with the label.
fn panel_field<'a>(shown: &'a str, label: &str) -> Option<&'a str> {
    shown
        .lines()
        .flat_map(|line| line.split(" | "))
        .find(|field| field.starts_with(&format!("{label}: ")))
}

/// Hovers the one button carrying the marker component, and no other.
fn hover_only<C: Component>(app: &mut App) {
    let hovered: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<C>>()
        .iter(app.world())
        .collect();
    assert_eq!(hovered.len(), 1, "one button of the kind on the card");
    let mut buttons = app.world_mut().query::<(Entity, &mut Interaction)>();
    for (button, mut interaction) in buttons.iter_mut(app.world_mut()) {
        *interaction = if button == hovered[0] {
            Interaction::Hovered
        } else {
            Interaction::None
        };
    }
}

/// The text on each skill button of the card, in query order.
fn skill_button_labels(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let children: Vec<Entity> = world
        .query::<(&hud::SkillButton, &Children)>()
        .iter(world)
        .flat_map(|(_, children)| children.iter().collect::<Vec<_>>())
        .collect();
    children
        .into_iter()
        .filter_map(|child| world.get::<Text>(child).map(|text| text.0.clone()))
        .collect()
}

/// How many warp buttons on the card stand lit.
fn open_warp_buttons(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<&hud::CardGate, With<hud::WarpButton>>()
        .iter(app.world())
        .filter(|gate| **gate == hud::CardGate::Open)
        .count()
}

/// A command card led by a warp gate of player 0 at (20, 20) that cools down
/// for good, beside a ready one at (26, 20), both powered, with 500 gold to
/// spend: the cooling gate's id, then the ready one's.
fn warp_card(app: &mut App) -> (SimulationId, SimulationId) {
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().init_resource::<input::WarpsSent>();
    let (cooling, cooling_id) =
        utils::create_entity(app.world_mut(), "warp_gate", utils::cell(20, 20), Some(0))
            .expect("the demo content defines a warp gate");
    let (_, ready_id) =
        utils::create_entity(app.world_mut(), "warp_gate", utils::cell(26, 20), Some(0))
            .expect("the demo content defines a warp gate");
    // A pylon powers both gates, which stand idle unpowered.
    utils::create_entity(app.world_mut(), "pylon", utils::cell(24, 23), Some(0))
        .expect("the demo content defines a pylon");
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 500);
    utils::run_ticks(app, 1);
    app.world_mut()
        .get_mut::<WarpComponent>(cooling)
        .expect("a warp gate keeps its cooldown")
        .ready_at = u32::MAX;
    app.world_mut().insert_resource(Leading(Some(cooling_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    (cooling_id, ready_id)
}
