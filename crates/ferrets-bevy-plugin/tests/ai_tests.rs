//! The scripted-AI frame sources: idle frames for unmanned slots, the think
//! cadence and its stagger, at-most-once thinking across blocked ticks, replay
//! gating, view building, and cross-run determinism.

mod utils;

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::{AiPlugin, AiRuntimes, game_view, install_ai_runtimes};
use ferrets_content::{entity_stats::EntityStatId, stats::ModifierOp};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_replay::{buffer::SharedBuffer, recorder::Recorder, replay::Replay};
use ferrets_script::{
    ai::view::content::ContentView,
    engine::{ScriptEngine, lua::LuaEngine},
};
use ferrets_simulation::{
    checksum::{self, CHECKSUM_INTERVAL},
    command::PlayerCommand,
    components::{
        hidden::Inside, order_queue::OrderQueueComponent, resource::ResourceSourceComponent,
    },
    entity_def,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    resources::PlayerResources,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, finish_policy::FinishPolicy,
        player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn::{self, Boarding},
};

//
// ─── Frame supply ─────────────────────────────────────────────────────────────
//

#[test]
fn ai_slots_without_runtimes_get_idle_frames_and_free_slots_get_none() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            None,
        ),
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
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            None,
        ),
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("orc"),
            None,
        ),
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
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            None,
        ),
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
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            None,
        ),
        PlayerSlot::free(2),
    ]);
    app.add_plugins(AiPlugin);
    install_ai(&mut app, &[(1, COUNTER)]);
    ferrets_bevy_plugin::replay::playback::install_per_game(app.world_mut(), empty_replay());
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
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            Some(1),
        ),
        PlayerSlot::occupied(2, PlayerType::Human, None, Some(1)),
    ]);
    utils::register_orders_content(&mut app);
    let world = app.world_mut();

    let (_, _) = utils::create_entity(world, "worker", utils::pos(5, 5), Some(1)).unwrap();
    let (hidden_own, _) = utils::create_entity(world, "worker", utils::pos(7, 5), Some(1)).unwrap();
    let (_, _) = utils::create_entity(world, "worker", utils::pos(10, 10), Some(0)).unwrap();
    let (hidden_enemy, _) =
        utils::create_entity(world, "worker", utils::pos(12, 10), Some(0)).unwrap();
    // The ally's, one of them hidden and both far from anything the brain
    // sees on its own.
    let (_, _) = utils::create_entity(world, "worker", utils::pos(25, 25), Some(2)).unwrap();
    let (hidden_ally, _) =
        utils::create_entity(world, "worker", utils::pos(27, 25), Some(2)).unwrap();
    let (mine, _) = utils::create_entity(world, "mine", utils::pos(2, 2), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 900;
    // The source takes no orders.
    world.entity_mut(mine).remove::<OrderQueueComponent>();
    for hidden in [hidden_own, hidden_enemy, hidden_ally] {
        spawn::hide_entity(world, hidden, Inside::Nothing);
    }
    world.resource_mut::<PlayerResources>().add(1, "gold", 120);

    let view = game_view(
        world,
        1,
        "human",
        AiVision::Omniscient,
        AiDetection::Detectors,
    );

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
    // The hidden one is listed where it went in.
    assert!(view.my_entities[1].hidden);
    assert_eq!((view.my_entities[1].x, view.my_entities[1].y), (7, 5));

    // An ally's entities are listed by ownership, the hidden one flagged, as
    // the brain's own are.
    assert_eq!(view.ally_entities.len(), 2);
    assert!(!view.ally_entities[0].hidden && view.ally_entities[1].hidden);

    // Hidden enemies are omitted; the neutral source exposes its remainder.
    assert_eq!(view.enemy_entities.len(), 1);
    assert_eq!(view.neutral_entities.len(), 1);
    assert_eq!(view.neutral_entities[0].resource_amount, Some(900));
    assert!(view.neutral_entities[0].health.is_none());
    // Taking no orders, the source is not idle, where the worker is.
    assert!(!view.neutral_entities[0].idle);
}

#[test]
fn game_view_names_borne_buffs_in_application_order() {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(
        0,
        PlayerType::Ai {
            vision: AiVision::Filtered,
            detection: AiDetection::Detectors,
        },
        Some("human"),
        None,
    )]);
    utils::register_orders_content(&mut app);
    let swift = utils::register_entity_buff(
        &mut app,
        "swift",
        EntityStatId::SPEED,
        ModifierOp::PercentAdd,
        "0.5",
        None,
    );
    let tough = utils::register_entity_buff(
        &mut app,
        "tough",
        EntityStatId::MAX_HEALTH,
        ModifierOp::FlatAdd,
        "5",
        None,
    );
    let (worker, _) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::create_owned(&mut app, "worker", 7, 5, 0);
    utils::apply_buff(app.world_mut(), worker, tough);
    utils::apply_buff(app.world_mut(), worker, swift);

    let view = game_view(
        app.world(),
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    assert_eq!(
        view.my_entities[0].buffs,
        vec!["tough".to_string(), "swift".to_string()]
    );
    assert!(view.my_entities[1].buffs.is_empty());
}

#[test]
fn game_view_rounds_fractional_supply_use_up() {
    let mut app = utils::supply_app();
    let lighter = utils::register_entity_buff(
        &mut app,
        "lighter",
        EntityStatId::SUPPLY_COST,
        ModifierOp::FlatAdd,
        "-0.5",
        None,
    );
    let (settler, _) = utils::create_owned(&mut app, "settler", 5, 5, 0);
    utils::apply_buff(app.world_mut(), settler, lighter);
    utils::run_ticks(&mut app, 1);

    // 1 − 0.5 = 0.5 supply used, read as 1 rather than 0.
    let view = game_view(
        app.world(),
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    assert_eq!(view.supply_used, 1);
}

#[test]
fn passenger_aboard_unseen_holder_of_another_side_is_listed_where_it_boarded() {
    let mut app = utils::transport_app();
    // A raft boards anyone; this one is player 2's, player 1's rival.
    let (raft, _) = utils::create_owned(&mut app, "raft", 10, 10, 2);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 1);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, raft),
        Boarding::Boarded
    );

    let view = game_view(
        app.world_mut(),
        1,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );

    // The brain keeps its own rifleman in view, where it boarded, not at the
    // rival raft it cannot sight.
    let rifleman = view
        .my_entities
        .iter()
        .find(|entity| entity.id == rifleman_id.0)
        .expect("a side keeps its own in view");
    assert!(rifleman.hidden);
    assert_eq!((rifleman.x, rifleman.y), (11, 10));
    assert!(view.enemy_entities.is_empty(), "the raft itself is unseen");
}

#[test]
fn passenger_aboard_sighted_holder_of_another_side_is_listed_where_holder_is() {
    let mut app = utils::transport_app();
    let (raft, _) = utils::create_owned(&mut app, "raft", 10, 10, 2);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 1);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, raft),
        Boarding::Boarded
    );
    // A second rifleman of the brain's side stands in sight of the raft.
    utils::create_owned(&mut app, "rifleman", 14, 10, 1);
    utils::run_ticks(&mut app, 1);

    let view = game_view(
        app.world_mut(),
        1,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );

    // The brain sees the raft, so it knows where its rifleman is carried.
    let rifleman = view
        .my_entities
        .iter()
        .find(|entity| entity.id == rifleman_id.0)
        .expect("a side keeps its own in view");
    assert!(rifleman.hidden);
    assert_eq!((rifleman.x, rifleman.y), (10, 10));
}

#[test]
fn sighted_holder_of_another_side_lists_no_passengers() {
    let mut app = utils::transport_app();
    let (raft, raft_id) = utils::create_owned(&mut app, "raft", 10, 10, 2);
    let (rider, _) = utils::create_owned(&mut app, "rifleman", 11, 10, 2);
    assert_eq!(
        spawn::board(app.world_mut(), rider, raft),
        Boarding::Boarded
    );
    utils::create_owned(&mut app, "rifleman", 14, 10, 1);
    utils::run_ticks(&mut app, 1);

    let view = game_view(
        app.world_mut(),
        1,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );

    // The raft is in sight; what rides inside it is not.
    let raft = view
        .enemy_entities
        .iter()
        .find(|entity| entity.id == raft_id.0)
        .expect("the raft stands in the brain's sight");
    assert!(raft.passengers.is_empty());
}

#[test]
fn worker_inside_own_walking_job_is_listed_where_job_is() {
    let mut app = utils::transport_app();
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (surgeon, surgeon_id) = utils::create_owned(&mut app, "surgeon", 12, 10, 0);
    utils::wound(&mut app, rifleman, "20");
    utils::select(&mut app, surgeon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: rifleman_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(entity_def::hidden(app.world(), surgeon));

    // The patient walks on with the surgeon inside it.
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 16),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);
    let patient_at = CellPos::from(entity_def::position(app.world(), rifleman));

    let view = game_view(
        app.world_mut(),
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    let listed = view
        .my_entities
        .iter()
        .find(|entity| entity.id == surgeon_id.0)
        .expect("a side keeps its own in view");
    assert!(listed.hidden);
    assert_ne!(patient_at, CellPos::new(12, 10));
    assert_eq!((listed.x, listed.y), (patient_at.x, patient_at.y));
}

#[test]
fn rider_of_holder_inside_unseen_job_is_listed_where_holder_went_in() {
    let mut app = utils::transport_app();
    let (seam, seam_id) =
        utils::create_entity(app.world_mut(), "ore_seam", utils::pos(10, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(seam)
        .unwrap()
        .amount = 100;
    let (hauler, hauler_id) = utils::create_owned(&mut app, "hauler", 9, 5, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 8, 5, 0);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, hauler),
        Boarding::Boarded
    );
    utils::select(&mut app, hauler_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: seam_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), hauler));

    // The hauler works inside a seam nobody of the brain's sights: both it and
    // its rider are listed where the hauler went in, (9, 5), not at the seam.
    let view = game_view(
        app.world_mut(),
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    let listed = |id: u32| {
        let entity = view
            .my_entities
            .iter()
            .find(|entity| entity.id == id)
            .expect("a side keeps its own in view");
        (entity.x, entity.y)
    };
    assert_eq!(listed(hauler_id.0), (9, 5));
    assert_eq!(listed(rifleman_id.0), (9, 5));
}

#[test]
fn carrier_waiting_beside_removed_source_is_listed_where_source_stood() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "lone_digger", 5, 5, 0);
    let (seam, seam_id) = utils::create_owned(&mut app, "roaming_seam", 9, 5, 0);
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(seam)
        .unwrap()
        .amount = 40;
    utils::create_owned(&mut app, "depot", 2, 4, 0);
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: seam_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    // The twenty-tick trip ends nineteen ticks on, boxed in.
    utils::run_ticks(&mut app, 19);
    assert!(entity_def::pending_reveal(app.world(), worker));
    spawn::destroy_entity(app.world_mut(), seam);
    utils::run_ticks(&mut app, 1);

    // The seam is gone; the digger waiting beside it is listed where its
    // return last searched, the seam's own cell (9, 5), not where it went in,
    // (8, 5).
    let view = game_view(
        app.world_mut(),
        0,
        "human",
        AiVision::Filtered,
        AiDetection::Detectors,
    );
    let digger = view
        .my_entities
        .iter()
        .find(|entity| entity.id == worker_id.0)
        .expect("a side keeps its own in view");
    assert!(digger.hidden);
    assert_eq!((digger.x, digger.y), (9, 5));
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
        detection = "detectors",
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
        detection = "detectors",
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
        detection = "detectors",
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
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("human"),
            None,
        ),
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
                detection: AiDetection::Detectors,
            },
            Some("orc"),
            None,
        ),
    ]);
    app.add_plugins(AiPlugin);
    utils::register_orders_content(&mut app);
    {
        let world = app.world_mut();
        utils::create_entity(world, "worker", utils::pos(4, 4), Some(1)).unwrap();
        utils::create_entity(world, "worker", utils::pos(20, 20), Some(2)).unwrap();
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
    let header = utils::skirmish_header(
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(
                1,
                PlayerType::Ai {
                    vision: AiVision::Filtered,
                    detection: AiDetection::Detectors,
                },
                Some("human"),
                None,
            ),
            PlayerSlot::free(2),
        ],
        FinishPolicy::Endless,
    );
    drop(Recorder::new(buffer.clone(), &header).expect("start recording"));
    Replay::read(buffer.bytes().as_slice()).expect("read replay")
}
