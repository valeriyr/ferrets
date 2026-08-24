//! Replay file IO for the demo: record every game to disk, and watch one back
//! through a native file-open dialog.
//!
//! The engine-side recording and playback systems live in `ferrets-bevy-plugin`; this
//! module supplies the files they read and write, and the menu/teardown plumbing
//! around them.

use std::{
    fs::File,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use ferrets_bevy_plugin::{ReplayPlayback, replay};
use ferrets_replay::{
    header::{RecordedGame, ReplayHeader},
    recorder::Recorder,
    replay::Replay,
};
use ferrets_simulation::{
    map::Map,
    session::{GameResult, GameSession},
};

use crate::{
    map, playback,
    render::SkillPulses,
    scenario::CurrentScenario,
    skirmish::CurrentSkirmish,
    states::{GameState, InGameUi},
};

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
    // The live map is what the game actually ran on, so it — not the menu's
    // settings — states the rules the recording is of.
    let map = world.resource::<Map>();
    let header = ReplayHeader::new(game, map.movement_model(), map.projection());

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
            replay::recorder::install_per_game(world, recorder);
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

    // Resolve the recorded game's content before touching the session, so an
    // unknown scenario or map leaves the menu alive. The same resolution the
    // headless runner uses, so the two cannot rebuild different games.
    let resolved = match playback::resolve(replay.header()) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };

    *world.resource_mut::<GameSession>() = playback::session(&resolved);
    ferrets_bevy_plugin::install_game_resources(world);
    ferrets_bevy_plugin::replay::playback::install_per_game(world, replay);
    // Hand a scenario to its scene spawner, so playback rebuilds exactly the
    // recorded scene from tick 0. No scenario runtime is installed — the
    // replay is the sole authority, and the win/loss check stands down during
    // playback.
    if let Some(scenario) = resolved.scenario {
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

    // The engine tears down its own per-game state — the simulation entities and
    // stores, the network session, recorder/playback, runtimes — so this exit
    // path need not know its roster; what `install_game_resources` clears at
    // the next game's start stays untouched here for the same reason.
    ferrets_bevy_plugin::teardown_game_resources(world);

    // Despawn the in-game HUD/overlay (sprites despawned with their sim entities).
    let ui: Vec<Entity> = world
        .query_filtered::<Entity, With<InGameUi>>()
        .iter(world)
        .collect();
    for entity in ui {
        world.despawn(entity);
    }

    // Despawning entities does not release the cells they occupied, so rebuild the
    // map to clear its occupation grid for the next game. The map is the game's
    // own to install, so it is the game's to rebuild.
    world.insert_resource(map::build());
    // Pulses are stamped with the tick they started on, and the next game's
    // ticks restart at zero — a survivor would draw a phantom ring on whatever
    // entity inherits its id.
    world.insert_resource(SkillPulses::default());

    world.remove_resource::<CurrentScenario>();
    world.remove_resource::<CurrentSkirmish>();
    world.remove_resource::<RecordingPath>();
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
