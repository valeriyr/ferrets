//! Replay file IO for the demo: record every game to disk, and watch one back
//! through a native file-open dialog.
//!
//! The engine-side recording and playback systems live in `ferrets-bevy-plugin`; this
//! module supplies the files they read and write, and the menu/teardown plumbing
//! around them.

use std::fs::File;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ferrets_bevy_plugin::{
    BlockedStreak, DesyncTracker, NetworkActive, NetworkSession, PauseIntent, PendingPause,
    ReplayPlayback, ReplayRecorder, install_game_resources, install_replay_playback,
    install_replay_recorder,
};
use ferrets_replay::header::{RecordedGame, ReplayHeader};
use ferrets_replay::recorder::Recorder;
use ferrets_replay::replay::Replay;
use ferrets_simulation::{
    entity_index::EntityIndex,
    session::{
        GameResult, GameSession, ai_hosting::AiHosting, authority::Authority,
        drop_policy::DropPolicy, finish_policy::FinishPolicy, player_slot,
    },
    simulation_id::SimulationIdGenerator,
};

use crate::map;
use crate::scenario::CurrentScenario;
use crate::skirmish::CurrentSkirmish;
use crate::states::{GameState, InGameUi};

/// Set by the menu to ask for a replay to be opened; consumed by
/// [`start_watching`].
#[derive(Resource)]
pub struct WatchReplayRequested;

/// Where the current game's replay is being written, kept so the path can be
/// reported when the game ends.
#[derive(Resource)]
struct RecordingPath(PathBuf);

/// Begins recording the game just started, unless it is itself a replay being
/// watched. Reads the configured session for the header. Runs on entering the
/// game.
pub fn start_recording(world: &mut World) {
    if world.get_resource::<ReplayPlayback>().is_some() {
        return;
    }

    // A scenario game is recorded by its name — the scenario defines the rest.
    // A skirmish has no name of its own, so its definition is embedded whole;
    // playback rebuilds either from content the game already knows.
    let game = match (
        world.get_resource::<CurrentScenario>(),
        world.get_resource::<CurrentSkirmish>(),
    ) {
        (Some(scenario), None) => RecordedGame::Scenario(scenario.0.name.clone()),
        (None, Some(skirmish)) => RecordedGame::Skirmish(skirmish.0.clone()),
        // Exactly one definition describes the game; recording is the
        // crash-safety net, so a game its entry path described ambiguously or
        // not at all cannot be recorded faithfully, and must not start.
        (Some(_), Some(_)) => panic!("a game was entered with both a scenario and a skirmish"),
        (None, None) => panic!("a game was entered with no scenario or skirmish installed"),
    };
    let header = ReplayHeader::new(game);

    let path = match new_replay_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("failed to prepare the replays directory: {error}");
            return;
        }
    };
    let file = match File::create(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("failed to create replay {}: {error}", path.display());
            return;
        }
    };
    match Recorder::new(file, &header) {
        Ok(recorder) => {
            install_replay_recorder(world, recorder);
            world.insert_resource(RecordingPath(path));
        }
        Err(error) => eprintln!("failed to start replay recording: {error}"),
    }
}

/// Opens a replay through the file dialog, configures the session from its
/// header, and enters the game in playback. Runs in the menu; a no-op unless a
/// watch was requested, and a no-op if the dialog is cancelled.
pub fn start_watching(world: &mut World) {
    if world.remove_resource::<WatchReplayRequested>().is_none() {
        return;
    }

    let Some(path) = rfd::FileDialog::new()
        .add_filter("ferrets replay", &["frep"])
        .set_directory(replays_dir())
        .pick_file()
    else {
        return;
    };

    let replay = match open_replay(&path) {
        Ok(replay) => replay,
        Err(error) => {
            eprintln!("failed to load replay {}: {error}", path.display());
            return;
        }
    };

    let current = ferrets_simulation::VERSION;
    if replay.header().engine_version != current {
        eprintln!(
            "warning: replay recorded by engine {} but this build is {current}; it may not replay faithfully",
            replay.header().engine_version,
        );
    }

    // Owned setup, taken before the replay is moved into playback.
    let game = replay.header().game.clone();

    // Resolve the recorded game's content before touching the session, so an
    // unknown scenario or map leaves the menu alive. A scenario defines its
    // own session; a skirmish spells its out in the recording.
    let (slots, map_name, finish_policy, scenario) = match game {
        RecordedGame::Scenario(name) => {
            let mission = crate::scenario::builtin_mission();
            if name != mission.name {
                eprintln!("the replay needs unknown scenario '{name}'");
                return;
            }
            (
                player_slot::scenario_slots(&mission),
                mission.map.name().to_string(),
                FinishPolicy::Scripted,
                Some(mission),
            )
        }
        RecordedGame::Skirmish(skirmish) => {
            if map::by_name(&skirmish.map).is_none() {
                eprintln!("the replay needs unknown map '{}'", skirmish.map);
                return;
            }
            (skirmish.slots, skirmish.map, skirmish.finish_policy, None)
        }
    };

    // The viewer is a spectator; follow the first occupied slot for the camera.
    let viewer = slots
        .iter()
        .find(|slot| slot.player_type().is_some())
        .map_or(0, |slot| slot.id());

    {
        let mut session = world.resource_mut::<GameSession>();
        // Playback never runs AI (the replay is the sole frame source), so
        // the hosting mode is irrelevant; the finish policy replays the
        // recorded game's.
        session.configure(
            viewer,
            slots,
            map_name,
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            finish_policy,
        );
    }
    install_game_resources(world);
    install_replay_playback(world, replay);
    // Hand a scenario to its scene spawner, so playback rebuilds exactly the
    // recorded scene from tick 0. No scenario runtime is installed — the
    // replay is the sole authority, and the win/loss check stands down during
    // playback.
    if let Some(scenario) = scenario {
        world.insert_resource(CurrentScenario(scenario));
    }
    world
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
}

/// Tears the game down when returning to the menu: reports the recorded replay,
/// despawns every simulation entity (their sprites go with them), and resets the
/// session and replay/network state to pending. Runs on leaving the game.
pub fn teardown_session(world: &mut World) {
    if let Some(path) = world.get_resource::<RecordingPath>() {
        let path = path.0.clone();
        let note = match world.resource::<GameSession>().result() {
            Some(GameResult::Desynchronization { tick }) => format!(" (desync at tick {tick})"),
            Some(GameResult::Aborted) => String::from(" (aborted)"),
            _ => String::new(),
        };
        println!("replay saved to {}{note}", path.display());
    }

    let entities: Vec<Entity> = {
        let index = world.resource::<EntityIndex>();
        index
            .all_entries()
            .into_iter()
            .map(|(_, entity)| entity)
            .collect()
    };
    for entity in entities {
        world.despawn(entity);
    }

    // Despawn the in-game HUD/overlay (sprites despawn with their sim entities above).
    let ui: Vec<Entity> = world
        .query_filtered::<Entity, With<InGameUi>>()
        .iter(world)
        .collect();
    for entity in ui {
        world.despawn(entity);
    }

    world.insert_resource(EntityIndex::default());
    world.insert_resource(SimulationIdGenerator::default());
    world.insert_resource(GameSession::pending());
    // Despawning entities does not release the cells they occupied, so rebuild the
    // map to clear its occupation grid for the next game.
    world.insert_resource(crate::map::build());

    world.remove_non_send_resource::<NetworkSession>();
    world.remove_resource::<NetworkActive>();
    ferrets_bevy_plugin::ai::remove_ai_runtimes(world);
    ferrets_bevy_plugin::remove_scenario_runtime(world);
    world.remove_resource::<CurrentScenario>();
    world.remove_resource::<CurrentSkirmish>();
    world.remove_non_send_resource::<ReplayRecorder>();
    world.remove_resource::<ReplayPlayback>();
    world.remove_resource::<RecordingPath>();

    // These network resources live for the app's lifetime, so reset (not remove)
    // their per-game state — stale checksums or a pending pause would otherwise
    // bleed into the next network game.
    world.insert_resource(DesyncTracker::default());
    world.insert_resource(BlockedStreak::default());
    world.insert_resource(PendingPause::default());
    world.insert_resource(PauseIntent::default());
}

/// Opens and reads a replay file.
fn open_replay(path: &PathBuf) -> ferrets_replay::Result<Replay> {
    let file = File::open(path)?;
    Replay::read(std::io::BufReader::new(file))
}

/// A fresh, timestamped replay path, creating the replays directory if needed.
fn new_replay_path() -> std::io::Result<PathBuf> {
    let dir = replays_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("{stamp}.frep")))
}

/// The directory replays are written to and opened from.
fn replays_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("replays")
}
