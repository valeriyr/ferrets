//! The scripted-AI frame sources: idle frames for unmanned slots, the think
//! cadence and its stagger, at-most-once thinking across blocked ticks, replay
//! gating, view building, and cross-run determinism.

mod utils;

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::{AiPlugin, AiRuntimes, game_view, install_ai_runtimes};
use ferrets_bevy_plugin::{install_replay_playback, spawn};
use ferrets_replay::buffer::SharedBuffer;
use ferrets_replay::{
    header::{RecordedGame, ReplayHeader},
    recorder::Recorder,
    replay::Replay,
};
use ferrets_script::ai::AiVision;
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::checksum;
use ferrets_simulation::{
    checksum::CHECKSUM_INTERVAL,
    command::PlayerCommand,
    components::{hidden::HiddenComponent, resource::ResourceSourceComponent},
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    resources::PlayerResources,
    session::{
        GameSession,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
    skirmish::Skirmish,
};

//
// ─── Frame supply ─────────────────────────────────────────────────────────────
//

#[test]
fn ai_slots_without_runtimes_get_idle_frames_and_free_slots_get_none() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
        PlayerSlot::free(2),
    ]);
    app.add_plugins(AiPlugin);
    app.world_mut().resource_mut::<GameSession>().start();

    utils::run_steps(&mut app, 10);

    let world = app.world_mut();
    assert_eq!(world.resource::<GameSession>().tick(), 10);
    let frames = world.resource::<InputFrames>().frames_in_range(5, 5);
    // The human (local input) and the brainless AI idle along; the free slot
    // contributes nothing — no tick requires its input.
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame.commands.is_empty()));
    assert!(frames.iter().all(|frame| frame.player != 2));
}

#[test]
fn ai_commands_land_only_on_staggered_think_ticks() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
        PlayerSlot::occupied(2, PlayerType::Ai, Some("orc"), None),
    ]);
    app.add_plugins(AiPlugin);
    install_ai(&mut app, &[(1, STOPPER), (2, STOPPER)]);
    app.world_mut().resource_mut::<GameSession>().start();

    utils::run_steps(&mut app, 12);

    // A frame targeting tick T was committed at tick T - SYNC_LATENCY; the
    // cadence is (tick + player * stagger) % period with stagger 5, period 4 —
    // so the two players' think ticks are disjoint.
    let frames = app
        .world_mut()
        .resource::<InputFrames>()
        .frames_in_range(SYNC_LATENCY, 11 + SYNC_LATENCY);
    for frame in frames {
        let source_tick = frame.tick - SYNC_LATENCY;
        let thinks = match frame.player {
            1 => (source_tick + 5).is_multiple_of(4),
            2 => (source_tick + 10).is_multiple_of(4),
            _ => continue,
        };
        assert_eq!(
            !frame.commands.is_empty(),
            thinks,
            "player {} at tick {source_tick}",
            frame.player
        );
    }
}

#[test]
fn blocked_ticks_do_not_rethink() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        // A remote human with no frame source: the loop blocks at tick 2.
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
        PlayerSlot::occupied(2, PlayerType::Ai, Some("human"), None),
    ]);
    app.add_plugins(AiPlugin);
    install_ai(&mut app, &[(2, COUNTER)]);
    app.world_mut().resource_mut::<GameSession>().start();

    utils::run_steps(&mut app, 12);

    {
        let world = app.world_mut();
        let session = world.resource::<GameSession>();
        assert_eq!(session.tick(), SYNC_LATENCY);
        assert!(session.is_blocked());
        // The counter thought once per tick 0..=2 despite ten re-runs at the
        // blocked tick — its third think (count 3) is what tick 4 carries. A
        // re-think would also trip the input-immutability assertion.
        let frames = world.resource::<InputFrames>().frames_in_range(4, 4);
        let ai_frame = frames.iter().find(|f| f.player == 2).expect("ai frame");
        assert_eq!(
            ai_frame.commands,
            vec![PlayerCommand::Move {
                target: utils::pos(3, 0),
                flush: true,
            }]
        );
    }

    // Supplying the missing remote frames resumes the loop.
    for tick in SYNC_LATENCY..=4 {
        app.world_mut()
            .resource_mut::<InputFrames>()
            .push_frame(PlayerFrame::idle(1, tick));
    }
    utils::run_steps(&mut app, 3);
    assert_eq!(app.world_mut().resource::<GameSession>().tick(), 5);
}

#[test]
fn replay_playback_gates_ai_sources_off() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
        PlayerSlot::free(2),
    ]);
    app.add_plugins(AiPlugin);
    install_ai(&mut app, &[(1, COUNTER)]);
    install_replay_playback(app.world_mut(), empty_replay());
    app.world_mut().resource_mut::<GameSession>().start();

    utils::run_steps(&mut app, 1);

    // Neither the AI nor the unmanned source committed anything past the
    // warmup: playback is the sole frame source.
    let frames = app.world().resource::<InputFrames>();
    assert!(!frames.has_frame(1, SYNC_LATENCY));
    assert!(!frames.has_frame(2, SYNC_LATENCY));
}

//
// ─── View building ────────────────────────────────────────────────────────────
//

#[test]
fn game_view_classifies_and_snapshots_entities() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
    ]);
    utils::register_orders_content(&mut app);
    let world = app.world_mut();

    let (_, _) = spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(1)).unwrap();
    let (hidden_own, _) = spawn::spawn_entity(world, "worker", utils::pos(7, 5), Some(1)).unwrap();
    let (_, _) = spawn::spawn_entity(world, "worker", utils::pos(10, 10), Some(0)).unwrap();
    let (hidden_enemy, _) =
        spawn::spawn_entity(world, "worker", utils::pos(12, 10), Some(0)).unwrap();
    let (mine, _) = spawn::spawn_entity(world, "mine", utils::pos(2, 2), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 900;
    world.entity_mut(hidden_own).insert(HiddenComponent);
    world.entity_mut(hidden_enemy).insert(HiddenComponent);
    world.resource_mut::<PlayerResources>().add(1, "gold", 120);

    let view = game_view(world, 1, "human", AiVision::Omniscient);

    assert_eq!(view.player, 1);
    assert_eq!(view.race, "human");
    assert_eq!((view.map_width, view.map_height), (32, 32));
    assert_eq!(view.resources, vec![("gold".to_string(), 120)]);

    // Own entities include the hidden one, flagged; ids ascend.
    assert_eq!(view.my_entities.len(), 2);
    assert!(view.my_entities[0].id < view.my_entities[1].id);
    let worker = &view.my_entities[0];
    assert_eq!((worker.x, worker.y), (5, 5));
    assert_eq!(worker.type_name, "worker");
    assert_eq!(worker.health, Some(20));
    assert!(worker.idle && !worker.hidden && !worker.under_construction);
    assert!(worker.carrying.is_none() && worker.resource_amount.is_none());
    assert!(view.my_entities[1].hidden);

    // Hidden enemies are omitted; the neutral source exposes its remainder.
    assert_eq!(view.enemy_entities.len(), 1);
    assert_eq!(view.neutral_entities.len(), 1);
    assert_eq!(view.neutral_entities[0].resource_amount, Some(900));
    assert!(view.neutral_entities[0].health.is_none());
}

//
// ─── Determinism ──────────────────────────────────────────────────────────────
//

#[test]
fn identical_ai_sessions_stay_checksum_identical() {
    let first = run_ai_session();
    let second = run_ai_session();

    assert!(first.len() >= 20, "expected a long checksum trail");
    assert_eq!(first, second);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Emits one command every think, on a short cadence.
const STOPPER: &str = r#"
    define_ai("stopper", {
        period = 4,
        vision = "filtered",
        think = function(state, view)
            return { { kind = "stop" } }
        end,
    })
"#;

/// Encodes its own think count into a command, exposing every extra think.
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

/// Marches its first unit between two columns, mutating real sim state.
const PATROL: &str = r#"
    define_ai("patrol", {
        period = 5,
        vision = "filtered",
        think = function(state, view)
            local unit = view.my_entities[1]
            if unit == nil then return end
            state.flip = not state.flip
            local x = state.flip and 3 or 12
            return {
                { kind = "select", id = unit.id },
                { kind = "move", x = x, y = unit.y },
            }
        end,
    })
"#;

/// Installs one runtime per `(player, script)` pair.
fn install_ai(app: &mut App, scripts: &[(PlayerId, &str)]) {
    let content = ContentView {
        resources: Vec::new(),
        entities: Vec::new(),
        researches: Vec::new(),
        skills: Vec::new(),
    };
    let mut runtimes = BTreeMap::new();
    for (player, script) in scripts {
        runtimes.insert(
            *player,
            LuaEngine.load_ai(script, &content).expect("load ai"),
        );
    }
    install_ai_runtimes(app.world_mut(), AiRuntimes(runtimes));
}

/// One full AI-vs-AI session, sampling the state checksum at every interval.
fn run_ai_session() -> Vec<u64> {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
        PlayerSlot::occupied(2, PlayerType::Ai, Some("orc"), None),
    ]);
    app.add_plugins(AiPlugin);
    utils::register_orders_content(&mut app);
    {
        let world = app.world_mut();
        spawn::spawn_entity(world, "worker", utils::pos(4, 4), Some(1)).unwrap();
        spawn::spawn_entity(world, "worker", utils::pos(20, 20), Some(2)).unwrap();
    }
    install_ai(&mut app, &[(1, PATROL), (2, PATROL)]);
    app.world_mut().resource_mut::<GameSession>().start();

    let mut checksums = Vec::new();
    for _ in 0..200 {
        app.world_mut().run_schedule(FixedUpdate);
        let world = app.world_mut();
        if world
            .resource::<GameSession>()
            .tick()
            .is_multiple_of(CHECKSUM_INTERVAL)
        {
            checksums.push(checksum::state_checksum(world));
        }
    }
    checksums
}

/// A replay with a matching header and no recorded ticks.
fn empty_replay() -> Replay {
    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
            PlayerSlot::free(2),
        ],
        map: "test".to_string(),
        finish_policy: FinishPolicy::Endless,
    }));
    drop(Recorder::new(buffer.clone(), &header).expect("start recording"));
    Replay::read(buffer.bytes().as_slice()).expect("read replay")
}
