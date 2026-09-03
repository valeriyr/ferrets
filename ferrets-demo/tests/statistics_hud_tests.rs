//! The end-of-game tallies beside the verdict: shown only once the session has
//! finished, and reporting what each player actually did.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_demo::hud::{self, FinalStatsText};
use ferrets_simulation::{
    events::SpendCause,
    movement_model::MovementModel,
    resources::{self, PlayerResources},
    session::{
        GameResult, GameSession, ai_hosting::AiHosting, ai_vision::AiVision, authority::Authority,
        drop_policy::DropPolicy, finish_policy::FinishPolicy, local_role::LocalRole,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    statistics::Statistics,
};

#[test]
fn tallies_stay_empty_while_game_runs() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);
    utils::run_ticks(&mut app, 2);

    assert_eq!(panel_text(&mut app), "", "a running game shows no summary");
}

#[test]
fn finished_game_reports_what_each_player_spent() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);

    // A charge the tallies will have folded by the time the panel is filled.
    let cost = ferrets_content::costs::cost([("gold", 25)]);
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 100);
    let world = app.world_mut();
    resources::charge(
        world,
        0,
        cost,
        SpendCause::Training {
            trainer: SimulationId(1),
        },
    );
    utils::run_ticks(&mut app, 2);
    assert_eq!(
        app.world().resource::<Statistics>().player(0).spent("gold"),
        25,
        "the charge is tallied in full before the game ends"
    );

    app.world_mut()
        .resource_mut::<GameSession>()
        .finish(GameResult::Draw);

    let shown = panel_text(&mut app);
    assert_eq!(
        shown.lines().filter(|line| line.starts_with('P')).count(),
        2,
        "one headline per player in the session: {shown:?}"
    );
    assert!(
        shown.contains("P0 (you): built 0, lost 0, killed 0, damage 0/0, research 0, skills 0"),
        "the headline reports every tally, not a summary: {shown:?}"
    );
    assert!(
        shown.contains("gold +0/-25"),
        "and the economy line reports what moved: {shown:?}"
    );
    assert!(
        shown.is_ascii(),
        "the panel stays in the character set the demo font covers: {shown:?}"
    );
}

#[test]
fn free_and_environment_seats_get_no_row() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    spawn_panel(&mut app);

    // A fuller roster: two seated players, an open seat, and an environment
    // combatant. The session and its tallies are swapped in over the fixture's.
    let mut session = GameSession::configured(
        LocalRole::Player(0),
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
            PlayerSlot::occupied(1, PlayerType::Human, Some("orc"), None),
            PlayerSlot::free(2),
            PlayerSlot::environment(3, AiVision::Omniscient),
        ],
        ferrets_demo::map::NAME,
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        FinishPolicy::Endless,
    );
    session.start();
    session.finish(GameResult::Draw);
    app.world_mut().insert_resource(session);
    app.world_mut().insert_resource(Statistics::new(4));

    let shown = panel_text(&mut app);
    assert_eq!(
        shown.lines().filter(|line| line.starts_with('P')).count(),
        2,
        "an open seat and an environment combatant get no row: {shown:?}"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Spawns the panel node the statistics system writes into, standing in for the
/// HUD setup a real game does on entering the scene.
fn spawn_panel(app: &mut App) {
    app.world_mut().spawn((FinalStatsText, Text::new("")));
}

/// Runs the statistics system and reads back what it wrote.
fn panel_text(app: &mut App) -> String {
    app.world_mut()
        .run_system_once(hud::update_final_statistics)
        .expect("the statistics system runs");
    let mut query = app.world_mut().query::<(&Text, &FinalStatsText)>();
    let (text, _) = query.single(app.world()).expect("one panel");
    text.0.clone()
}
