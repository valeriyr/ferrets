//! The AI runtime: scripts declare a brain with `define_ai`, observe integer
//! snapshots, and return command tables that round-trip to player commands;
//! malformed scripts and results surface as errors rather than panics.

use ferrets_math::FixedU64;
use ferrets_math::fixed_urect::FixedURect;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_script::ai::AiRuntime;
use ferrets_script::ai::view::content::{ContentView, EntityContentView};
use ferrets_script::ai::view::game::{EntityView, GameView};
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_script::error::ScriptError;
use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::simulation_id::SimulationId;

//
// ─── Round-trip ─────────────────────────────────────────────────────────────
//

#[test]
fn think_returns_commands_as_player_commands() {
    let source = r#"
        define_ai("all_kinds", {
            period = 20,
            think = function(state, view)
                return {
                    { kind = "select", id = 7 },
                    { kind = "select_area", x1 = 1, y1 = 2, x2 = 3, y2 = 4 },
                    { kind = "move", x = 5, y = 6 },
                    { kind = "attack", target = 8, flush = false },
                    { kind = "send", target = 9 },
                    { kind = "train", trainer = 10, type_name = "peasant" },
                    { kind = "build", builder = 11, type_name = "barracks", x = 12, y = 13 },
                    { kind = "stop" },
                }
            end,
        })
    "#;
    let mut runtime = load_ai(source, &empty_content()).expect("load ai");

    let commands = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        commands,
        vec![
            PlayerCommand::SelectById {
                id: SimulationId(7)
            },
            PlayerCommand::SelectByRect {
                rect: FixedURect::from_corners(
                    cell(1, 2),
                    FixedUVec2::new(
                        FixedU64::from_num(4) - FixedU64::DELTA,
                        FixedU64::from_num(5) - FixedU64::DELTA,
                    ),
                ),
            },
            PlayerCommand::Move {
                target: cell(5, 6),
                flush: true,
            },
            PlayerCommand::Attack {
                target: SimulationId(8),
                flush: false,
            },
            PlayerCommand::SendToEntity {
                target: SimulationId(9),
                flush: true,
            },
            PlayerCommand::TrainEntity {
                trainer: SimulationId(10),
                type_name: "peasant".to_string(),
            },
            PlayerCommand::BuildEntity {
                builder: SimulationId(11),
                type_name: "barracks".to_string(),
                position: cell(12, 13),
                flush: true,
            },
            PlayerCommand::Stop,
        ]
    );
}

#[test]
fn returning_nil_or_empty_table_yields_no_commands() {
    let nothing = r#"
        define_ai("mute", { period = 1, think = function(state, view) end })
    "#;
    let empty = r#"
        define_ai("empty", { period = 1, think = function(state, view) return {} end })
    "#;

    let mut from_nil = load_ai(nothing, &empty_content()).expect("load ai");
    let mut from_empty = load_ai(empty, &empty_content()).expect("load ai");

    assert!(from_nil.think(&empty_view()).expect("think").is_empty());
    assert!(from_empty.think(&empty_view()).expect("think").is_empty());
}

#[test]
fn accepts_integral_floats_in_command_fields() {
    // Integer division in a script yields floats; whole values must pass.
    let source = r#"
        define_ai("divider", {
            period = 1,
            think = function(state, view)
                return { { kind = "move", x = 10 / 2, y = 0 } }
            end,
        })
    "#;
    let mut runtime = load_ai(source, &empty_content()).expect("load ai");

    let commands = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        commands,
        vec![PlayerCommand::Move {
            target: cell(5, 0),
            flush: true,
        }]
    );
}

//
// ─── Persistent state ───────────────────────────────────────────────────────
//

#[test]
fn state_persists_across_think_calls() {
    let mut runtime = load_ai(COUNTER, &empty_content()).expect("load ai");

    let first = runtime.think(&empty_view()).expect("think");
    let second = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        first,
        vec![PlayerCommand::Move {
            target: cell(1, 0),
            flush: true,
        }]
    );
    assert_eq!(
        second,
        vec![PlayerCommand::Move {
            target: cell(2, 0),
            flush: true,
        }]
    );
}

#[test]
fn two_runtimes_from_one_script_have_independent_state() {
    let mut first = load_ai(COUNTER, &empty_content()).expect("load ai");
    let mut second = load_ai(COUNTER, &empty_content()).expect("load ai");

    // Advance the first runtime twice; its state (and its globals) must not
    // leak into the second.
    first.think(&empty_view()).expect("think");

    let first_commands = first.think(&empty_view()).expect("think");
    assert_eq!(
        first_commands,
        vec![PlayerCommand::Move {
            target: cell(2, 0),
            flush: true,
        }]
    );

    let second_commands = second.think(&empty_view()).expect("think");
    assert_eq!(
        second_commands,
        vec![PlayerCommand::Move {
            target: cell(1, 0),
            flush: true,
        }]
    );
}

//
// ─── Definition contract ────────────────────────────────────────────────────
//

#[test]
fn reports_missing_define_ai_as_ai_error() {
    let Err(error) = load_ai("local x = 1", &empty_content()) else {
        panic!("must reject");
    };

    assert!(
        matches!(&error, ScriptError::AiError(m) if m == "script must call define_ai"),
        "got {error:?}"
    );
}

#[test]
fn reports_second_define_ai_as_ai_error() {
    let source = r#"
        define_ai("first", { period = 1, think = function() end })
        define_ai("second", { period = 1, think = function() end })
    "#;

    let Err(error) = load_ai(source, &empty_content()) else {
        panic!("must reject");
    };

    assert!(
        matches!(&error, ScriptError::AiError(m) if m == "define_ai must be called exactly once"),
        "got {error:?}"
    );
}

#[test]
fn accepts_integral_float_period() {
    let source = r#"
        define_ai("divided", { period = 60 / 3, think = function() end })
    "#;

    let runtime = load_ai(source, &empty_content()).expect("load ai");

    assert_eq!(runtime.period(), 20);
}

#[test]
fn reports_invalid_period_as_ai_error() {
    let source = r#"
        define_ai("hasty", { period = 0, think = function() end })
    "#;

    let Err(error) = load_ai(source, &empty_content()) else {
        panic!("must reject");
    };

    assert!(
        matches!(&error, ScriptError::AiError(m) if m.contains("'period' must be a positive integer")),
        "got {error:?}"
    );
}

#[test]
fn reports_non_function_think_as_ai_error() {
    let source = r#"
        define_ai("static", { period = 1, think = 5 })
    "#;

    let Err(error) = load_ai(source, &empty_content()) else {
        panic!("must reject");
    };

    assert!(
        matches!(&error, ScriptError::AiError(m) if m.contains("'think' must be a function")),
        "got {error:?}"
    );
}

#[test]
fn exposes_name_and_period() {
    let source = r#"
        define_ai("named", { period = 20, think = function() end })
    "#;

    let runtime = load_ai(source, &empty_content()).expect("load ai");

    assert_eq!(runtime.name(), "named");
    assert_eq!(runtime.period(), 20);
}

//
// ─── Command validation ─────────────────────────────────────────────────────
//

#[test]
fn reports_unknown_kind_as_command_error() {
    let error = think_error(r#"return { { kind = "dance" } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 1: unknown kind 'dance'"),
        "got {error:?}"
    );
}

#[test]
fn reports_missing_field_as_command_error() {
    let error = think_error(r#"return { { kind = "train", trainer = 3 } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("field 'type_name'")),
        "got {error:?}"
    );
}

#[test]
fn reports_fractional_number_as_command_error() {
    let error = think_error(r#"return { { kind = "move", x = 1.5, y = 0 } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("1.5 is not a whole number")),
        "got {error:?}"
    );
}

#[test]
fn malformed_element_fails_the_whole_batch() {
    // A valid command before the bad one must not survive the batch.
    let source = r#"
        define_ai("mixed", {
            period = 1,
            think = function(state, view)
                return { { kind = "stop" }, { kind = "nope" } }
            end,
        })
    "#;
    let mut runtime = load_ai(source, &empty_content()).expect("load ai");

    let error = runtime.think(&empty_view()).expect_err("must reject");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 2: unknown kind 'nope'"),
        "got {error:?}"
    );
}

#[test]
fn returning_a_non_table_is_a_command_error() {
    let error = think_error("return 42");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("think must return a command array or nil")),
        "got {error:?}"
    );
}

//
// ─── Runtime errors and sandbox ─────────────────────────────────────────────
//

#[test]
fn think_error_surfaces_and_runtime_stays_usable() {
    let source = r#"
        define_ai("fragile", {
            period = 1,
            think = function(state, view)
                if view.tick == 1 then error("boom") end
                return {}
            end,
        })
    "#;
    let mut runtime = load_ai(source, &empty_content()).expect("load ai");

    let error = runtime.think(&view_at_tick(1)).expect_err("must fail");
    let recovered = runtime.think(&view_at_tick(2));

    assert!(
        matches!(&error, ScriptError::EngineError(m) if m.contains("boom")),
        "got {error:?}"
    );
    assert!(recovered.is_ok(), "got {recovered:?}");
}

#[test]
fn ambient_state_stdlib_is_unavailable() {
    let source = r#"
        define_ai("sandboxed", {
            period = 1,
            think = function(state, view)
                if os ~= nil or io ~= nil then error("stdlib leaked") end
                if pcall(math.random) then error("math.random available") end
                if pcall(math.randomseed, 7) then error("math.randomseed available") end
                return {}
            end,
        })
    "#;
    let mut runtime = load_ai(source, &empty_content()).expect("load ai");

    assert!(runtime.think(&empty_view()).is_ok());
}

//
// ─── Determinism and view shape ─────────────────────────────────────────────
//

#[test]
fn identical_views_produce_identical_command_sequences() {
    let source = r#"
        define_ai("mirror", {
            period = 1,
            think = function(state, view)
                state.round = (state.round or 0) + 1
                local commands = {}
                for _, e in ipairs(view.my_entities) do
                    if e.idle then
                        commands[#commands + 1] = { kind = "select", id = e.id }
                        commands[#commands + 1] = { kind = "move", x = e.x + state.round, y = e.y }
                    end
                end
                return commands
            end,
        })
    "#;
    let mut first = load_ai(source, &empty_content()).expect("load ai");
    let mut second = load_ai(source, &empty_content()).expect("load ai");

    for tick in 0..3 {
        let view = populated_view(tick);
        let a = first.think(&view).expect("think");
        let b = second.think(&view).expect("think");
        assert_eq!(a, b, "diverged at tick {tick}");
        assert!(!a.is_empty());
    }
}

#[test]
fn scripts_read_view_and_content_tables() {
    let source = r#"
        define_ai("reader", {
            period = 1,
            think = function(state, view)
                if view.race ~= "human" then error("race") end
                if view.map.width ~= 64 then error("map width") end
                local hall = view.my_entities[1]
                if hall.type_name ~= "town_hall" then error("type_name") end
                if hall.health ~= 800 then error("health") end
                if hall.train_queue[1] ~= "peasant" then error("train_queue") end
                if hall.under_construction then error("under_construction") end
                local mine = view.neutral_entities[1]
                if mine.resource_amount ~= 900 then error("resource_amount") end
                local worker = content.entities.peasant
                if worker.cost[1].kind ~= "gold" then error("cost kind") end
                if worker.train_time ~= 40 then error("train_time") end
                if not worker.can_move then error("can_move") end
                if content.resources[1] ~= "gold" then error("resources") end
                local gold = view.resources.gold
                return { { kind = "move", x = gold + worker.cost[1].amount, y = hall.x } }
            end,
        })
    "#;
    let mut runtime = load_ai(source, &demo_like_content()).expect("load ai");

    let commands = runtime.think(&populated_view(0)).expect("think");

    assert_eq!(
        commands,
        vec![PlayerCommand::Move {
            target: cell(120 + 50, 8),
            flush: true,
        }]
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Loads a brain through the engine the suite runs against — the only line
/// naming a binding.
fn load_ai(source: &str, content: &ContentView) -> ferrets_script::Result<Box<dyn AiRuntime>> {
    LuaEngine.load_ai(source, content)
}

/// A brain that moves by its own think count, exercising persistent state.
const COUNTER: &str = r#"
    define_ai("counter", {
        period = 1,
        think = function(state, view)
            state.count = (state.count or 0) + 1
            return { { kind = "move", x = state.count, y = 0 } }
        end,
    })
"#;

fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::from(NavPos::new(x, y))
}

fn empty_content() -> ContentView {
    ContentView {
        resources: Vec::new(),
        entities: Vec::new(),
    }
}

/// One worker type with the fields the reader script checks.
fn demo_like_content() -> ContentView {
    ContentView {
        resources: vec!["gold".to_string(), "wood".to_string()],
        entities: vec![EntityContentView {
            name: "peasant".to_string(),
            cost: vec![("gold".to_string(), 50)],
            train_time: Some(40),
            build_time: None,
            trains: None,
            builds: Some(vec!["town_hall".to_string()]),
            size: (1, 1),
            health: Some(30),
            attack: None,
            harvests: Some(vec!["gold".to_string()]),
            stores: None,
            can_move: true,
        }],
    }
}

fn empty_view() -> GameView {
    view_at_tick(0)
}

fn view_at_tick(tick: u32) -> GameView {
    GameView {
        tick,
        player: 1,
        race: "human".to_string(),
        map_width: 64,
        map_height: 64,
        resources: Vec::new(),
        my_entities: Vec::new(),
        enemy_entities: Vec::new(),
        neutral_entities: Vec::new(),
    }
}

/// A view with a producing hall, two workers (one idle), and a gold mine.
fn populated_view(tick: u32) -> GameView {
    GameView {
        tick,
        player: 1,
        race: "human".to_string(),
        map_width: 64,
        map_height: 64,
        resources: vec![("gold".to_string(), 120), ("wood".to_string(), 40)],
        my_entities: vec![
            EntityView {
                id: 1,
                type_name: "town_hall".to_string(),
                x: 8,
                y: 9,
                health: Some(800),
                idle: false,
                hidden: false,
                carrying: None,
                train_queue: vec!["peasant".to_string()],
                under_construction: false,
                resource_amount: None,
            },
            EntityView {
                id: 2,
                type_name: "peasant".to_string(),
                x: 10,
                y: 9,
                health: Some(30),
                idle: true,
                hidden: false,
                carrying: Some(("gold".to_string(), 3)),
                train_queue: Vec::new(),
                under_construction: false,
                resource_amount: None,
            },
        ],
        enemy_entities: Vec::new(),
        neutral_entities: vec![EntityView {
            id: 3,
            type_name: "gold_mine".to_string(),
            x: 4,
            y: 4,
            health: None,
            idle: true,
            hidden: false,
            carrying: None,
            train_queue: Vec::new(),
            under_construction: false,
            resource_amount: Some(900),
        }],
    }
}

/// Loads a one-expression think body and returns its first think error.
fn think_error(body: &str) -> ScriptError {
    let source = format!(
        r#"
        define_ai("failing", {{
            period = 1,
            think = function(state, view)
                {body}
            end,
        }})
        "#
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");
    runtime.think(&empty_view()).expect_err("must reject")
}
