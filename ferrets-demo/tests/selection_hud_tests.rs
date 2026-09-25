//! The selection HUD: what the panel reports and withholds about the one entity
//! picked, and the command card rebuilding to one button of each kind.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::PendingInput;
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::{
    hud::{self, SelectionText},
    input::{Inspected, Leading},
    render::{ObserverPerspective, Sighted},
};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{order_queue::OrderQueueComponent, train::TrainQueueComponent},
    movement_model::MovementModel,
    order::Order,
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
    app
}
