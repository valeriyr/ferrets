//! The selection HUD: what the panel reports and withholds about the one entity
//! picked, and the command card rebuilding to one button of each kind.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::PendingInput;
use ferrets_content::{
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
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_demo::{
    hud::{self, SelectionText, SupplyText},
    input::{self, AimHover, AimVerdict, InputMode, Inspected, Leading, TargetedOrder},
    render::{ObserverPerspective, Sighted},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillTarget},
    components::{
        build::{self, SiteWork},
        entity_skills::SkillsComponent,
        last_hit,
        order_queue::OrderQueueComponent,
        pools,
        train::TrainQueueComponent,
    },
    entity_index::EntityIndex,
    game_loop::{self, buffs::Bearing},
    movement_model::MovementModel,
    order::Order,
    player_research::PlayerResearch,
    resources::PlayerResources,
    session::GameSession,
    supply,
    visibility::Sighting,
};

//
// ─── Selection panel ────────────────────────────────────────────────────────
//

#[test]
fn panel_reports_live_values_of_pick_in_sight() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let shown = panel_text(&mut app);
    assert!(shown.contains("Grunt"), "the pick is named: {shown:?}");
    assert!(
        shown.contains("HP "),
        "a pick in sight reports its health: {shown:?}"
    );
}

#[test]
fn panel_reports_energy_against_its_maximum() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (_, archer) =
        utils::create_entity(app.world_mut(), "archer", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines an archer");
    utils::select(&mut app, archer, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(archer)
        .expect("the archer stands");
    pools::drain(app.world_mut(), entity, PoolId::ENERGY, utils::fixed("35"));

    // 60 − 35 = 25 of the archer's 60.
    let shown = panel_text(&mut app);
    assert!(
        shown.contains("energy 25/60"),
        "the energy reads against its maximum: {shown:?}"
    );
}

#[test]
fn panel_names_site_and_buff_outage_as_simulation_judges_them() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (grunt, grunt_id) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a grunt");
    utils::select(&mut app, grunt_id, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    build::mark_as_site(
        app.world_mut(),
        grunt,
        SiteWork::Crew {
            builders: Default::default(),
        },
    );
    let shown = panel_text(&mut app);
    assert!(
        shown.contains("under construction"),
        "a site reads under construction: {shown:?}"
    );

    build::mark_as_built(app.world_mut(), grunt);
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
        game_loop::buffs::apply_entity_buff(app.world_mut(), grunt, stunned),
        Bearing::Borne
    );
    let shown = panel_text(&mut app);
    assert!(
        shown.contains("disabled"),
        "a buff outage reads disabled: {shown:?}"
    );
}

#[test]
fn supply_readout_rounds_and_turns_red_on_what_it_shows() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    app.world_mut()
        .spawn((SupplyText, Text::new(""), TextColor::default()));
    utils::create_entity(app.world_mut(), "farm", utils::at_cell(24, 24), Some(0))
        .expect("the demo content defines a farm");
    let (grunt, _) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
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
        utils::create_entity(app.world_mut(), "gateway", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a gateway");
    utils::select(&mut app, gateway, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let shown = panel_text(&mut app);
    assert!(
        shown.contains("disabled"),
        "an unpowered gateway reads disabled: {shown:?}"
    );
}

#[test]
fn panel_withholds_live_values_once_rival_pick_leaves_sight() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    // A rival's unit, lit by a scout of the side's own: what the side does not
    // own, it reads only while it makes it out.
    utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(22, 20), Some(1))
            .expect("the demo content defines a grunt");
    utils::run_ticks(&mut app, 2);
    utils::select(&mut app, grunt, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // The stamp the sighting pass would write for a lit rival, by hand: the
    // panel reads the stamp, and an entity never stamped reads as unseen.
    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Seen));
    assert!(panel_text(&mut app).contains("HP "));

    app.world_mut()
        .entity_mut(entity)
        .insert(Sighted(Sighting::Unseen));

    let shown = panel_text(&mut app);
    assert!(
        shown.contains("out of sight"),
        "the panel says the side has lost it: {shown:?}"
    );
    assert!(
        !shown.contains("HP "),
        "health is not read off an entity the side cannot see: {shown:?}"
    );
    assert!(
        !shown.contains("speed"),
        "nor is anything else live: {shown:?}"
    );
}

#[test]
fn glimpsed_rival_pick_withholds_live_values_too() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
        .expect("the demo content defines a grunt");
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(22, 20), Some(1))
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
    assert!(shown.contains("out of sight"), "{shown:?}");
    assert!(!shown.contains("HP "), "{shown:?}");
}

#[test]
fn panel_reports_own_pick_that_carries_no_sighting() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    let (entity, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
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
        shown.contains("HP "),
        "a side reads its own wherever they are: {shown:?}"
    );
    assert!(
        !shown.contains("out of sight"),
        "and is never told it has lost them: {shown:?}"
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
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(20, 20), Some(0))
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
    let (_, blacksmith) = utils::create_entity(
        app.world_mut(),
        "blacksmith",
        utils::at_cell(26, 26),
        Some(0),
    )
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
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(20, 20), Some(0))
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
    let (_, blacksmith) = utils::create_entity(
        app.world_mut(),
        "blacksmith",
        utils::at_cell(24, 24),
        Some(0),
    )
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
    let (entity, blacksmith) = utils::create_entity(
        app.world_mut(),
        "blacksmith",
        utils::at_cell(24, 24),
        Some(0),
    )
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
fn cancel_buttons_gray_out_with_nothing_to_call_off() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));

    // A barracks with nothing queued: the cancel-unit button is raised with
    // the card and grayed by the availability pass, since there is nothing
    // to call off; one entry queued brings it back.
    let (entity, barracks) =
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(20, 20), Some(0))
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
    let (entity, blacksmith) = utils::create_entity(
        app.world_mut(),
        "blacksmith",
        utils::at_cell(26, 26),
        Some(0),
    )
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
    // and says what it costs — 150 gold, 100 wood and 90 ticks, 4.5 s.
    let (_, factory) =
        utils::create_entity(app.world_mut(), "factory", utils::at_cell(20, 20), Some(0))
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
            "150 gold, 100 wood, 4.5 s".to_string(),
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
        utils::create_entity(app.world_mut(), "archer", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines an archer");
    app.world_mut().insert_resource(Leading(Some(archer_id)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    // Battle focus: 30 energy, and 80 ticks of cooldown, 4.0 s.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "30 energy, 4.0 s cooldown");

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
        .start_cooldown(battle_focus, 0);
    build::mark_as_site(
        app.world_mut(),
        archer,
        SiteWork::Crew {
            builders: Default::default(),
        },
    );
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Still under construction");

    build::mark_as_built(app.world_mut(), archer);
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

    // Led by something that knows no battle focus, or by nothing at all.
    let (_, grunt_id) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(24, 20), Some(0))
            .expect("the demo content defines a grunt");
    app.world_mut().insert_resource(Leading(Some(grunt_id)));
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "It cannot cast this");
    app.world_mut().insert_resource(Leading(None));
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "Nothing selected can cast it");
}

#[test]
fn armed_skill_names_why_it_refuses_hovered_target() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    let (_, shaman_id) =
        utils::create_entity(app.world_mut(), "shaman", utils::at_cell(30, 30), Some(0))
            .expect("the demo content defines a shaman");
    let (_, barracks_id) =
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(20, 20), Some(0))
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
    let (_, grunt) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a grunt");
    app.world_mut().insert_resource(Leading(Some(grunt)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::SkillButton>(&mut app);

    // Blood rite: 10 gold and 8 health, and 160 ticks of cooldown, 8.0 s.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "10 gold, 8 health, 8.0 s cooldown");
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

    // War drums: 50 gold, and 300 ticks of cooldown, 15.0 s.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "50 gold, 15.0 s cooldown");
}

#[test]
fn hovered_build_buttons_name_their_price_and_time() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 1000);
    let (_, drone) =
        utils::create_entity(app.world_mut(), "drone", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a drone");
    app.world_mut().insert_resource(Leading(Some(drone)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    // A spawning pit is 200 gold and 100 wood in 120 ticks, 6.0 s; a tumor 25
    // gold in 40, 2.0 s; a hatchery 400 gold in 200, 10.0 s.
    assert_eq!(
        hover_each::<hud::BuildButton>(&mut app),
        vec![
            "200 gold, 100 wood, 6.0 s".to_string(),
            "25 gold, 2.0 s".to_string(),
            "400 gold, 10.0 s".to_string(),
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
    let (_, tower) = utils::create_entity(
        app.world_mut(),
        "watch_tower",
        utils::at_cell(20, 20),
        Some(0),
    )
    .expect("the demo content defines a watch tower");
    utils::select(&mut app, tower, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    app.world_mut().insert_resource(Leading(Some(tower)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");
    hover_only::<hud::MorphButton>(&mut app);

    // Into a guard tower: 80 gold and 20 wood.
    recolor_card(&mut app);
    assert_eq!(hint_text(&mut app), "80 gold, 20 wood");
}

#[test]
fn grayed_button_names_unmet_part_of_requirement_tree() {
    let mut app = card_app();
    app.world_mut().spawn((hud::CommandCard, Node::default()));
    app.world_mut().spawn((hud::CardHint, Text::new("")));
    register_totem(&mut app);
    let (totem, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::at_cell(20, 20), Some(0))
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
    // health reads 34%; 40 ticks idle is 2.0 s, 200 unhurt 10.0 s.
    recolor_card(&mut app);
    assert_eq!(
        hint_text(&mut app),
        "Needs (a Spawning Pit or Frenzy Ritual) and health under 34% and energy at least 50 and Sight Range at least 99 and standing idle and 2.0 s idle and 10.0 s unhurt"
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
    let (_, blacksmith) = utils::create_entity(
        app.world_mut(),
        "blacksmith",
        utils::at_cell(26, 26),
        Some(0),
    )
    .expect("the demo content defines a blacksmith");
    app.world_mut().insert_resource(Leading(Some(blacksmith)));
    app.world_mut()
        .run_system_once(hud::update_command_card)
        .expect("the command card system runs");

    // Iron weapons: 100 gold, 50 wood and 200 ticks, 10.0 s; vitality drill:
    // 80 gold, 40 wood and 160 ticks, 8.0 s.
    assert_eq!(
        hover_each::<hud::ResearchButton>(&mut app),
        vec![
            "100 gold, 50 wood, 10.0 s".to_string(),
            "80 gold, 40 wood, 8.0 s".to_string(),
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
    let mut app = utils::demo_map_app(MovementModel::Cell);
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
    let ground = {
        let registry = app.world().resource::<ContentRegistry>();
        registry.layer("ground").expect("the demo declares ground")
    };
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    let frenzy_ritual = registry
        .research("frenzy_ritual")
        .expect("the demo content defines the frenzy ritual");
    registry.register(
        EntityTypeDef::new("totem")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(40))
            .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(5))
            .with_morphs([MorphTransition::new(
                "watch_tower",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(10),
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
