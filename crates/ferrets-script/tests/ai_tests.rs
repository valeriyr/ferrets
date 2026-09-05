//! The AI runtime: scripts declare a brain with `define_ai`, observe integer
//! snapshots, and return command tables that round-trip to player commands;
//! malformed scripts and results surface as errors rather than panics.

use ferrets_content::{
    costs,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    player_buffs::PlayerBuffDef,
    registry::ContentRegistry,
    research::{ResearchDef, ResearchId},
    skills::{EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillDef},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::nav_grid::LayerId;
use ferrets_script::{
    ai::{
        AiRuntime,
        view::{
            content::{AttackView, ContentView, EntityContentView, MorphView},
            game::{EntityView, GameView},
        },
    },
    engine::{ScriptEngine, lua::LuaEngine},
    error::ScriptError,
};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{rally::RallyTarget, stance::Stance},
    order::AttackTarget,
    session::ai_vision::AiVision,
    simulation_id::SimulationId,
};

//
// ─── Round-trip ─────────────────────────────────────────────────────────────
//

#[test]
fn think_returns_commands_as_player_commands() {
    let source = ai_script(
        r#"function(state, view)
            return {
                { kind = "select", id = 7 },
                { kind = "select_area", x1 = 1, y1 = 2, x2 = 3, y2 = 4 },
                { kind = "move", x = 5, y = 6 },
                { kind = "attack", target = 8, flush = false },
                { kind = "send", target = 9 },
                { kind = "train", trainer = 10, type_name = "peasant" },
                { kind = "build", builder = 11, type_name = "barracks", x = 12, y = 13 },
                { kind = "rally", entity = 14, x = 15, y = 16 },
                { kind = "rally", entity = 17, target = 18 },
                { kind = "rally", entity = 19 },
                { kind = "attack_move", x = 20, y = 21 },
                { kind = "patrol", x = 22, y = 23, flush = false },
                { kind = "guard", target = 24 },
                { kind = "stance", stance = "stand_ground" },
                { kind = "morph", type_name = "gryphon_aloft", flush = false },
                { kind = "stop" },
            }
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

    let commands = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        commands,
        vec![
            PlayerCommand::SelectById {
                id: SimulationId(7),
                mode: SelectMode::Replace,
            },
            PlayerCommand::SelectByRect {
                rect: FixedURect::from_corners(
                    cell(1, 2),
                    FixedUVec2::new(
                        FixedU64::from_num(4) - FixedU64::DELTA,
                        FixedU64::from_num(5) - FixedU64::DELTA,
                    ),
                ),
                mode: SelectMode::Replace,
            },
            PlayerCommand::Move {
                target: cell(5, 6),
                flush: true,
            },
            PlayerCommand::Attack {
                target: AttackTarget::Entity(SimulationId(8)),
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
            PlayerCommand::SetRallyPoint {
                entity: SimulationId(14),
                target: Some(RallyTarget::Position(cell(15, 16))),
            },
            PlayerCommand::SetRallyPoint {
                entity: SimulationId(17),
                target: Some(RallyTarget::Entity(SimulationId(18))),
            },
            PlayerCommand::SetRallyPoint {
                entity: SimulationId(19),
                target: None,
            },
            PlayerCommand::AttackMove {
                target: cell(20, 21),
                flush: true,
            },
            PlayerCommand::Patrol {
                target: cell(22, 23),
                flush: false,
            },
            PlayerCommand::Guard {
                target: SimulationId(24),
                flush: true,
            },
            PlayerCommand::SetStance {
                stance: Stance::StandGround,
            },
            PlayerCommand::Morph {
                type_name: "gryphon_aloft".to_string(),
                flush: false,
            },
            PlayerCommand::Stop,
        ]
    );
}

#[test]
fn research_command_resolves_name_to_handle() {
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "research", researcher = 5, research = "smithing" } }
        end"#,
    );
    let (content, smithing) = research_content();
    let mut runtime = load_ai(&source, &content).expect("load ai");

    let commands = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        commands,
        vec![PlayerCommand::StartResearch {
            researcher: SimulationId(5),
            research: smithing,
        }]
    );
}

#[test]
fn unknown_research_name_is_command_error() {
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "research", researcher = 5, research = "alchemy" } }
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

    let error = runtime.think(&empty_view()).expect_err("must fail");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("unknown research 'alchemy'")),
        "got {error:?}"
    );
}

#[test]
fn use_skill_command_resolves_name_and_caster() {
    let source = ai_script(
        r#"function(state, view)
            return {
                { kind = "use_skill", skill = "battle_focus", caster = 12 },
                { kind = "use_skill", skill = "second_wind", caster = 9, target = 4 },
                { kind = "use_skill", skill = "war_drums", caster = "player" },
            }
        end"#,
    );
    let (content, _) = research_content();
    let skill_id = |name: &str| {
        content
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .expect("skill listed")
            .id
    };
    let mut runtime = load_ai(&source, &content).expect("load ai");

    let commands = runtime.think(&empty_view()).expect("think");

    assert_eq!(
        commands,
        vec![
            PlayerCommand::UseSkill {
                skill: skill_id("battle_focus"),
                caster: SkillCasterRef::Entity(SimulationId(12)),
                target: None,
            },
            PlayerCommand::UseSkill {
                skill: skill_id("second_wind"),
                caster: SkillCasterRef::Entity(SimulationId(9)),
                target: Some(SkillTarget::Entity(SimulationId(4))),
            },
            PlayerCommand::UseSkill {
                skill: skill_id("war_drums"),
                caster: SkillCasterRef::Player,
                target: None,
            },
        ]
    );
}

#[test]
fn unknown_skill_name_is_command_error() {
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "use_skill", skill = "meteor", caster = 5 } }
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

    let error = runtime.think(&empty_view()).expect_err("must fail");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("unknown skill 'meteor'")),
        "got {error:?}"
    );
}

#[test]
fn use_skill_without_caster_is_command_error() {
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "use_skill", skill = "war_drums" } }
        end"#,
    );
    let (content, _) = research_content();
    let mut runtime = load_ai(&source, &content).expect("load ai");

    let error = runtime.think(&empty_view()).expect_err("must fail");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m.contains("field 'caster': missing")),
        "got {error:?}"
    );
}

#[test]
fn scripts_read_skill_catalogue() {
    let source = ai_script(
        r#"function(state, view)
            local focus = content.skills.battle_focus
            if focus.caster ~= "entity" then error("wrong caster") end
            if focus.target ~= "caster" then error("wrong target") end
            if focus.requires[1] ~= "smithing" then error("wrong requires") end
            local drums = content.skills.war_drums
            if drums.caster ~= "player" then error("wrong drums caster") end
            if drums.target ~= nil then error("drums take no target") end
            if drums.requires ~= nil then error("drums require nothing") end
            local lab = content.entities.lab
            if lab.skills[1] ~= "battle_focus" then error("wrong lab skills") end
            return {}
        end"#,
    );
    let (content, _) = research_content();
    let mut runtime = load_ai(&source, &content).expect("load ai");

    assert!(runtime.think(&empty_view()).is_ok());
}

#[test]
fn scripts_read_research_catalogue_and_state() {
    let source = ai_script(
        r#"function(state, view)
            local smithing = content.researches.smithing
            if smithing.cost.gold ~= 30 then error("wrong cost") end
            if smithing.time ~= 200 then error("wrong time") end
            if smithing.requires[1] ~= "lab" then error("wrong requires") end
            if view.researched[1] ~= "smithing" then error("wrong researched") end
            if view.researching[1] ~= "tactics" then error("wrong researching") end
            return {}
        end"#,
    );
    let (content, _) = research_content();
    let mut runtime = load_ai(&source, &content).expect("load ai");

    let mut view = empty_view();
    view.researched = vec!["smithing".to_string()];
    view.researching = vec!["tactics".to_string()];

    assert!(runtime.think(&view).is_ok());
}

#[test]
fn returning_nil_or_empty_table_yields_no_commands() {
    let nothing = ai_script("function(state, view) end");
    let empty = ai_script("function(state, view) return {} end");

    let mut from_nil = load_ai(&nothing, &empty_content()).expect("load ai");
    let mut from_empty = load_ai(&empty, &empty_content()).expect("load ai");

    assert!(from_nil.think(&empty_view()).expect("think").is_empty());
    assert!(from_empty.think(&empty_view()).expect("think").is_empty());
}

#[test]
fn accepts_integral_floats_in_command_fields() {
    // Integer division in a script yields floats; whole values must pass.
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "move", x = 10 / 2, y = 0 } }
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

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
        define_ai("first", { period = 1, vision = "filtered", think = function() end })
        define_ai("second", { period = 1, vision = "filtered", think = function() end })
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
        define_ai("divided", { period = 60 / 3, vision = "filtered", think = function() end })
    "#;

    let runtime = load_ai(source, &empty_content()).expect("load ai");

    assert_eq!(runtime.period(), 20);
}

#[test]
fn requires_explicit_vision_and_reads_declaration() {
    // No principled default exists, so omitting `vision` is an error rather than
    // a silent guess; a declaration is read back verbatim.
    let missing = r#"
        define_ai("fair", { period = 1, think = function() end })
    "#;
    let filtered = r#"
        define_ai("scout", { period = 1, vision = "filtered", think = function() end })
    "#;
    let omniscient = r#"
        define_ai("cheater", { period = 1, vision = "omniscient", think = function() end })
    "#;

    let Err(error) = load_ai(missing, &empty_content()) else {
        panic!("must reject a definition with no vision");
    };
    assert!(
        matches!(&error, ScriptError::AiError(m) if m.contains("must declare 'vision'")),
        "got {error:?}"
    );
    assert_eq!(
        load_ai(filtered, &empty_content())
            .expect("load ai")
            .vision(),
        AiVision::Filtered
    );
    assert_eq!(
        load_ai(omniscient, &empty_content())
            .expect("load ai")
            .vision(),
        AiVision::Omniscient
    );
}

#[test]
fn reports_invalid_vision_as_ai_error() {
    let source = r#"
        define_ai("confused", { period = 1, vision = "wallhack", think = function() end })
    "#;

    let Err(error) = load_ai(source, &empty_content()) else {
        panic!("must reject");
    };

    assert!(
        matches!(&error, ScriptError::AiError(m) if m.contains("'vision' must be 'filtered' or 'omniscient'")),
        "got {error:?}"
    );
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
        define_ai("named", { period = 20, vision = "filtered", think = function() end })
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
fn reports_unknown_stance_as_command_error() {
    let error = think_error(r#"return { { kind = "stance", stance = "berserk" } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 1: field 'stance': unknown stance 'berserk'"),
        "got {error:?}"
    );
}

#[test]
fn reports_rally_with_target_and_cell_as_command_error() {
    let error =
        think_error(r#"return { { kind = "rally", entity = 1, target = 2, x = 3, y = 4 } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 1: rally takes either a target or a cell, not both"),
        "got {error:?}"
    );
}

#[test]
fn reports_rally_with_half_cell_as_command_error() {
    let error = think_error(r#"return { { kind = "rally", entity = 1, x = 3 } }"#);

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 1: rally cell needs both x and y"),
        "got {error:?}"
    );
}

#[test]
fn malformed_element_fails_whole_batch() {
    // A valid command before the bad one must not survive the batch.
    let source = ai_script(
        r#"function(state, view)
            return { { kind = "stop" }, { kind = "nope" } }
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

    let error = runtime.think(&empty_view()).expect_err("must reject");

    assert!(
        matches!(&error, ScriptError::CommandError(m) if m == "element 2: unknown kind 'nope'"),
        "got {error:?}"
    );
}

#[test]
fn returning_non_table_is_command_error() {
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
    let source = ai_script(
        r#"function(state, view)
            if view.tick == 1 then error("boom") end
            return {}
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

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
    let source = ai_script(
        r#"function(state, view)
            if os ~= nil or io ~= nil then error("stdlib leaked") end
            if pcall(math.random) then error("math.random available") end
            if pcall(math.randomseed, 7) then error("math.randomseed available") end
            return {}
        end"#,
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");

    assert!(runtime.think(&empty_view()).is_ok());
}

//
// ─── Determinism and view shape ─────────────────────────────────────────────
//

#[test]
fn identical_views_produce_identical_command_sequences() {
    let source = ai_script(
        r#"function(state, view)
            state.round = (state.round or 0) + 1
            local commands = {}
            for _, e in ipairs(view.my_entities) do
                if e.idle then
                    commands[#commands + 1] = { kind = "select", id = e.id }
                    commands[#commands + 1] = { kind = "move", x = e.x + state.round, y = e.y }
                end
            end
            return commands
        end"#,
    );
    let mut first = load_ai(&source, &empty_content()).expect("load ai");
    let mut second = load_ai(&source, &empty_content()).expect("load ai");

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
    let source = ai_script(
        r#"function(state, view)
            if view.race ~= "human" then error("race") end
            if view.map.width ~= 64 then error("map width") end
            local hall = view.my_entities[1]
            if hall.type_name ~= "town_hall" then error("type_name") end
            if hall.health ~= 800 then error("health") end
            if hall.train_queue[1] ~= "peasant" then error("train_queue") end
            if hall.under_construction then error("under_construction") end
            if hall.stance ~= nil then error("hall stance") end
            if view.my_entities[2].stance ~= "flee" then error("worker stance") end
            local mine = view.neutral_entities[1]
            if mine.resource_amount ~= 900 then error("resource_amount") end
            local worker = content.entities.peasant
            if worker.cost[1].kind ~= "gold" then error("cost kind") end
            if worker.train_time ~= 40 then error("train_time") end
            if not worker.can_move then error("can_move") end
            if worker.max_health ~= 30 then error("max_health") end
            if worker.morphs[1].into ~= "town_hall" then error("morph into") end
            if worker.morphs[1].cost[1].amount ~= 400 then error("morph cost") end
            if worker.morphs[1].time ~= 200 then error("morph time") end
            local soldier = content.entities.soldier
            if soldier.morphs ~= nil then error("soldier morphs") end
            if soldier.attack.damage ~= 10 then error("attack damage") end
            if soldier.attack.attack_range ~= 1 then error("attack range") end
            if content.resources[1] ~= "gold" then error("resources") end
            local gold = view.resources.gold
            return { { kind = "move", x = gold + worker.cost[1].amount, y = hall.x } }
        end"#,
    );
    let mut runtime = load_ai(&source, &demo_like_content()).expect("load ai");

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

/// A loadable one-brain script wrapping `think` (a Lua `function(state, view) …
/// end` literal) with a fixed name, period, and filtered vision, so a test
/// states only the behaviour it exercises. Tests that pin a specific period,
/// name, or vision spell out their own `define_ai`.
fn ai_script(think: &str) -> String {
    format!(r#"define_ai("test", {{ period = 1, vision = "filtered", think = {think} }})"#)
}

/// A brain that moves by its own think count, exercising persistent state.
const COUNTER: &str = r#"
    define_ai("counter", {
        period = 1,
        vision = "filtered",
        think = function(state, view)
            state.count = (state.count or 0) + 1
            return { { kind = "move", x = state.count, y = 0 } }
        end,
    })
"#;

fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::from(CellPos::new(x, y))
}

/// A content view built from a real registry holding the `smithing` research
/// (30 gold, 200 ticks, requires "lab") and the free `tactics` unlock, plus
/// the handle `smithing` resolves to. The registry also carries three skills —
/// `battle_focus` (entity-cast on itself, requires `smithing`), `second_wind`
/// (entity-cast on an ally), and the player-cast `war_drums` — carried by the
/// `lab` type where a type is needed.
fn research_content() -> (ContentView, ResearchId) {
    let mut registry = ContentRegistry::default();
    registry.register_layer("ground");
    registry.register_resource("gold");
    let smithing = registry.register_research(
        "smithing",
        ResearchDef::new(costs::cost([("gold", 30)]), 200, None, ["lab"]),
    );
    registry.register_research(
        "tactics",
        ResearchDef::new(costs::Cost::new(), 100, None, Vec::<String>::new()),
    );
    let battle_focus = registry.register_skill(
        "battle_focus",
        SkillDef {
            cooldown: 5,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: vec!["smithing".to_string()],
        },
    );
    let second_wind = registry.register_skill(
        "second_wind",
        SkillDef {
            cooldown: 5,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Ally,
                effect: EntityCastEffect::Heal(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    let drums = registry.register_player_buff(
        "drums_haste",
        PlayerBuffDef {
            player_modifiers: Vec::new(),
            entity_modifiers: vec![EntityModifier {
                stat: EntityStatId::SPEED,
                op: ModifierOp::PercentAdd,
                magnitude: FixedI64::ONE,
            }],
            duration: Some(10),
            stack_rule: StackRule::Refresh,
        },
    );
    registry.register_skill(
        "war_drums",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Player {
                cost: costs::Cost::new(),
                effect: PlayerCastEffect::ApplyBuff(drums),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("lab")
            .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
            .with_researcher([smithing])
            .with_energy(50, FixedU64::ONE)
            .with_skills([battle_focus, second_wind]),
    );
    registry.validate();
    (ContentView::from_registry(&registry), smithing)
}

fn empty_content() -> ContentView {
    ContentView {
        resources: Vec::new(),
        entities: Vec::new(),
        researches: Vec::new(),
        skills: Vec::new(),
    }
}

/// One worker type with the fields the reader script checks.
fn demo_like_content() -> ContentView {
    ContentView {
        resources: vec!["gold".to_string(), "wood".to_string()],
        entities: vec![
            EntityContentView {
                name: "peasant".to_string(),
                cost: vec![("gold".to_string(), 50)],
                train_time: Some(40),
                build_time: None,
                trains: None,
                builds: Some(vec!["town_hall".to_string()]),
                size: (1, 1),
                max_health: Some(30),
                attack: None,
                harvests: Some(vec!["gold".to_string()]),
                stores: None,
                can_move: true,
                researches: None,
                skills: None,
                requires: None,
                morphs: Some(vec![MorphView {
                    into: "town_hall".to_string(),
                    cost: vec![("gold".to_string(), 400)],
                    time: Some(200),
                }]),
            },
            EntityContentView {
                name: "soldier".to_string(),
                cost: vec![("gold".to_string(), 100)],
                train_time: Some(20),
                build_time: None,
                trains: None,
                builds: None,
                size: (1, 1),
                max_health: Some(50),
                attack: Some(AttackView {
                    damage: 10,
                    attack_range: 1,
                }),
                harvests: None,
                stores: None,
                can_move: true,
                researches: None,
                skills: None,
                requires: None,
                morphs: None,
            },
        ],
        researches: Vec::new(),
        skills: Vec::new(),
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
        supply_provided: 0,
        supply_used: 0,
        researched: Vec::new(),
        researching: Vec::new(),
        my_entities: Vec::new(),
        ally_entities: Vec::new(),
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
        supply_provided: 0,
        supply_used: 0,
        researched: Vec::new(),
        researching: Vec::new(),
        my_entities: vec![
            EntityView {
                id: 1,
                type_name: "town_hall".to_string(),
                x: 8,
                y: 9,
                health: Some(800),
                energy: None,
                damage: None,
                armor: None,
                idle: false,
                hidden: false,
                carrying: None,
                train_queue: vec!["peasant".to_string()],
                under_construction: false,
                disabled: false,
                stance: None,
                resource_amount: None,
                boarded: None,
                passengers: Vec::new(),
            },
            EntityView {
                id: 2,
                type_name: "peasant".to_string(),
                x: 10,
                y: 9,
                health: Some(30),
                energy: None,
                damage: None,
                armor: None,
                idle: true,
                hidden: false,
                carrying: Some(("gold".to_string(), 3)),
                train_queue: Vec::new(),
                under_construction: false,
                disabled: false,
                stance: Some("flee".to_string()),
                resource_amount: None,
                boarded: None,
                passengers: Vec::new(),
            },
        ],
        ally_entities: Vec::new(),
        enemy_entities: Vec::new(),
        neutral_entities: vec![EntityView {
            id: 3,
            type_name: "gold_mine".to_string(),
            x: 4,
            y: 4,
            health: None,
            energy: None,
            damage: None,
            armor: None,
            idle: true,
            hidden: false,
            carrying: None,
            train_queue: Vec::new(),
            under_construction: false,
            disabled: false,
            stance: None,
            resource_amount: Some(900),
            boarded: None,
            passengers: Vec::new(),
        }],
    }
}

/// Loads a one-expression think body and returns its first think error.
fn think_error(body: &str) -> ScriptError {
    let source = format!(
        r#"
        define_ai("failing", {{
            period = 1,
            vision = "filtered",
            think = function(state, view)
                {body}
            end,
        }})
        "#
    );
    let mut runtime = load_ai(&source, &empty_content()).expect("load ai");
    runtime.think(&empty_view()).expect_err("must reject")
}
