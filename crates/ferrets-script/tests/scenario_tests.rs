//! The scenario runtime: scripts declare objectives and a win/loss `check` with
//! `define_scenario`, observe the same integer snapshots as an AI brain, and
//! report per-objective progress plus an outcome; malformed scripts and results
//! surface as errors rather than panics.

use ferrets_script::ai::view::content::ContentView;
use ferrets_script::ai::view::game::{EntityView, GameView};
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_script::error::ScriptError;
use ferrets_script::scenario::{ObjectiveStatus, Outcome, ScenarioRuntime, ScenarioStatus};

/// A build-an-army scenario: win with a finished barracks and three archers,
/// lose if every unit is gone. Mirrors the demo mission.
const BUILD_ARMY: &str = r#"
    define_scenario("build_army", {
        period = 10,
        objectives = {
            { id = "barracks", label = "Build a barracks" },
            { id = "archers", label = "Train 3 archers" },
        },
        check = function(state, view)
            local barracks, archers = 0, 0
            for _, entity in ipairs(view.my_entities) do
                if entity.type_name == "barracks" and not entity.under_construction then
                    barracks = barracks + 1
                elseif entity.type_name == "archer" then
                    archers = archers + 1
                end
            end
            local built, trained = barracks >= 1, archers >= 3
            local outcome = "ongoing"
            if built and trained then
                outcome = "victory"
            elseif #view.my_entities == 0 then
                outcome = "defeat"
            end
            return {
                objectives = { barracks = built, archers = trained },
                outcome = outcome,
            }
        end,
    })
"#;

//
// ─── Objectives and outcome ─────────────────────────────────────────────────
//

#[test]
fn evaluate_reports_objectives_in_declared_order() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");

    let status = runtime.evaluate(&view_with(Vec::new())).expect("evaluate");

    let ids: Vec<&str> = status.objectives.iter().map(|o| o.id.as_str()).collect();
    let labels: Vec<&str> = status.objectives.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(ids, vec!["barracks", "archers"]);
    assert_eq!(labels, vec!["Build a barracks", "Train 3 archers"]);
}

#[test]
fn evaluate_reports_victory_when_all_objectives_are_met() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");

    let status = runtime
        .evaluate(&view_with(vec![
            entity(1, "barracks", false),
            entity(2, "archer", false),
            entity(3, "archer", false),
            entity(4, "archer", false),
        ]))
        .expect("evaluate");

    assert_eq!(
        status,
        ScenarioStatus {
            objectives: vec![
                ObjectiveStatus {
                    id: "barracks".to_string(),
                    label: "Build a barracks".to_string(),
                    done: true,
                },
                ObjectiveStatus {
                    id: "archers".to_string(),
                    label: "Train 3 archers".to_string(),
                    done: true,
                },
            ],
            outcome: Outcome::Victory,
        }
    );
}

#[test]
fn evaluate_reports_ongoing_with_partial_progress() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");

    // A finished barracks but only two archers, plus a barracks still building.
    let status = runtime
        .evaluate(&view_with(vec![
            entity(1, "barracks", false),
            entity(2, "barracks", true),
            entity(3, "archer", false),
            entity(4, "archer", false),
        ]))
        .expect("evaluate");

    assert_eq!(status.outcome, Outcome::Ongoing);
    assert!(status.objectives[0].done, "barracks objective met");
    assert!(!status.objectives[1].done, "archers objective not yet met");
}

#[test]
fn evaluate_reports_defeat_when_no_entities_remain() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");

    let status = runtime.evaluate(&view_with(Vec::new())).expect("evaluate");

    assert_eq!(status.outcome, Outcome::Defeat);
    assert!(status.objectives.iter().all(|o| !o.done));
}

#[test]
fn unbuilt_barracks_does_not_count() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");

    let status = runtime
        .evaluate(&view_with(vec![entity(1, "barracks", true)]))
        .expect("evaluate");

    assert!(!status.objectives[0].done, "under-construction barracks");
    assert_eq!(status.outcome, Outcome::Ongoing);
}

#[test]
fn evaluation_is_deterministic() {
    let mut runtime = load_scenario(BUILD_ARMY, &empty_content()).expect("load scenario");
    let entities = || vec![entity(1, "barracks", false), entity(2, "archer", false)];

    let first = runtime.evaluate(&view_with(entities())).expect("evaluate");
    let second = runtime.evaluate(&view_with(entities())).expect("evaluate");

    assert_eq!(first, second);
}

//
// ─── Declaration errors ─────────────────────────────────────────────────────
//

#[test]
fn script_that_declares_no_scenario_errors() {
    let Err(error) = load_scenario("local x = 1", &empty_content()) else {
        panic!("must reject");
    };
    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m == "script must call define_scenario"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn declaring_two_scenarios_errors() {
    let source = format!("{BUILD_ARMY}\n{BUILD_ARMY}");
    let Err(error) = load_scenario(&source, &empty_content()) else {
        panic!("must reject");
    };
    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m == "define_scenario must be called exactly once"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn non_function_check_errors() {
    let source = r#"
        define_scenario("bad", {
            period = 10,
            objectives = {},
            check = 7,
        })
    "#;
    let Err(error) = load_scenario(source, &empty_content()) else {
        panic!("must reject");
    };
    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m.contains("'check' must be a function")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn non_positive_period_errors() {
    let source = r#"
        define_scenario("bad", {
            period = 0,
            objectives = {},
            check = function() return {} end,
        })
    "#;
    let Err(error) = load_scenario(source, &empty_content()) else {
        panic!("must reject");
    };
    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m.contains("'period' must be a positive integer")),
        "unexpected error: {error:?}"
    );
}

//
// ─── Result errors ──────────────────────────────────────────────────────────
//

#[test]
fn unknown_outcome_errors() {
    let source = r#"
        define_scenario("bad", {
            period = 10,
            objectives = {},
            check = function() return { outcome = "win" } end,
        })
    "#;
    let mut runtime = load_scenario(source, &empty_content()).expect("load scenario");

    let error = runtime.evaluate(&view_with(Vec::new())).unwrap_err();

    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m.contains("\"win\"")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn non_boolean_objective_errors() {
    let source = r#"
        define_scenario("bad", {
            period = 10,
            objectives = { { id = "done", label = "Done" } },
            check = function() return { objectives = { done = 1 } } end,
        })
    "#;
    let mut runtime = load_scenario(source, &empty_content()).expect("load scenario");

    let error = runtime.evaluate(&view_with(Vec::new())).unwrap_err();

    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m.contains("objective 'done' must be a boolean")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn non_table_result_errors() {
    let source = r#"
        define_scenario("bad", {
            period = 10,
            objectives = {},
            check = function() return 7 end,
        })
    "#;
    let mut runtime = load_scenario(source, &empty_content()).expect("load scenario");

    let error = runtime.evaluate(&view_with(Vec::new())).unwrap_err();

    assert!(
        matches!(&error, ScriptError::ScenarioError(m) if m.contains("check must return a table")),
        "unexpected error: {error:?}"
    );
}

//
// ─── Determinism hardening ──────────────────────────────────────────────────
//

#[test]
fn math_random_is_unavailable() {
    let source = r#"
        define_scenario("bad", {
            period = 10,
            objectives = {},
            check = function() return { outcome = "ongoing" } end,
        })
        math.random()
    "#;
    let Err(error) = load_scenario(source, &empty_content()) else {
        panic!("must reject");
    };
    assert!(
        matches!(&error, ScriptError::EngineError(m) if m.contains("math.random is unavailable")),
        "unexpected error: {error:?}"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

fn load_scenario(
    source: &str,
    content: &ContentView,
) -> ferrets_script::Result<Box<dyn ScenarioRuntime>> {
    LuaEngine.load_scenario(source, content)
}

fn empty_content() -> ContentView {
    ContentView {
        resources: Vec::new(),
        entities: Vec::new(),
    }
}

fn entity(id: u32, type_name: &str, under_construction: bool) -> EntityView {
    EntityView {
        id,
        type_name: type_name.to_string(),
        x: 0,
        y: 0,
        health: Some(1),
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
        my_entities,
        ally_entities: Vec::new(),
        enemy_entities: Vec::new(),
        neutral_entities: Vec::new(),
    }
}
