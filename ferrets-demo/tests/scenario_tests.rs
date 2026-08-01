//! The shipped story mission: its script loads against the real demo content
//! and resolves to victory once a barracks and three archers are fielded,
//! defeat once everything is gone.

use ferrets_demo::content::CONTENT;
use ferrets_demo::scenario::builtin_mission;
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::ai::view::game::{EntityView, GameView};
use ferrets_script::content;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_script::scenario::Outcome;

#[test]
fn mission_wins_with_barracks_and_three_archers() {
    let mut runtime = load_mission();

    let status = runtime
        .evaluate(&view_with(vec![
            entity("barracks", false),
            entity("archer", false),
            entity("archer", false),
            entity("archer", false),
        ]))
        .expect("evaluate");

    assert_eq!(status.outcome, Outcome::Victory);
    assert!(status.objectives.iter().all(|objective| objective.done));
}

#[test]
fn mission_is_ongoing_before_army_is_ready() {
    let mut runtime = load_mission();

    let status = runtime
        .evaluate(&view_with(vec![
            entity("town_hall", false),
            entity("barracks", false),
            entity("archer", false),
        ]))
        .expect("evaluate");

    assert_eq!(status.outcome, Outcome::Ongoing);
}

#[test]
fn mission_is_lost_when_everything_is_gone() {
    let mut runtime = load_mission();

    let status = runtime.evaluate(&view_with(Vec::new())).expect("evaluate");

    assert_eq!(status.outcome, Outcome::Defeat);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Loads the mission against a `ContentView` built from the real demo content,
/// proving script and content agree.
fn load_mission() -> Box<dyn ferrets_script::scenario::ScenarioRuntime> {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let view = ContentView::from_registry(&registry);
    LuaEngine
        .load_scenario(&builtin_mission().script, &view)
        .expect("mission loads")
}

fn entity(type_name: &str, under_construction: bool) -> EntityView {
    EntityView {
        id: 1,
        type_name: type_name.to_string(),
        x: 0,
        y: 0,
        health: Some(1),
        damage: None,
        armor: None,
        idle: true,
        hidden: false,
        carrying: None,
        train_queue: Vec::new(),
        under_construction,
        stance: None,
        resource_amount: None,
    }
}

fn view_with(my_entities: Vec<EntityView>) -> GameView {
    GameView {
        tick: 0,
        player: 0,
        race: "human".to_string(),
        map_width: 32,
        map_height: 32,
        resources: Vec::new(),
        supply_provided: 0,
        supply_used: 0,
        my_entities,
        ally_entities: Vec::new(),
        enemy_entities: Vec::new(),
        neutral_entities: Vec::new(),
    }
}
