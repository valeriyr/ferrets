//! Recording a game and replaying it: the determinism net over the demo's real
//! content and maps.
//!
//! No recording is committed to the repository. A `.frep` file carries the
//! checksums the build that wrote it produced, so a stored one would fail on
//! every deliberate change to the simulation instead of on a regression. These
//! tests record a game, replay that recording, and hold the engine to its own
//! output.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::{PauseIntent, ReplayPlugin, Seek, Step, replay};
use ferrets_demo::{input, playback, time::NOMINAL_TICK_HZ};
use ferrets_geometry::projection::Projection;
use ferrets_replay::{
    buffer::SharedBuffer,
    header::{RecordedGame, ReplayHeader},
    recorder::Recorder,
    replay::Replay,
};
use ferrets_simulation::{
    checksum::CHECKSUM_INTERVAL,
    command::{PlayerCommand, SelectMode},
    components::location::LocationComponent,
    entity_index::EntityIndex,
    movement_model::MovementModel,
    session::{GameSession, finish_policy::FinishPolicy},
    simulation_id::SimulationId,
};

#[test]
fn recorded_game_replays_to_identical_state() {
    let Recorded {
        replay,
        recorded,
        started,
        walked,
    } = record_walking_game(MovementModel::Continuous);
    let checksums: Vec<u32> = (0..=recorded)
        .filter(|&tick| replay.checksum_at(tick).is_some())
        .collect();
    let mut app = playback_app(replay);

    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert!(report.done, "played every recorded tick");
    assert_eq!(
        report.mismatch, None,
        "the replayed simulation diverged from the recording",
    );
    // The verification above only means something if checksums were recorded and
    // the units actually moved, so hold the fixture to both. Recording captures
    // every tick that completed, and a checksum rides every interval-th one.
    assert_eq!(recorded, TICKS - 1, "every played tick was recorded");
    let expected: Vec<u32> = (0..=recorded)
        .filter(|tick| tick.is_multiple_of(CHECKSUM_INTERVAL))
        .collect();
    assert_eq!(
        checksums, expected,
        "a checksum every {CHECKSUM_INTERVAL} ticks"
    );
    assert_ne!(walked, started, "the recorded game moved its units");
    // Every position, not just the checksummed ticks: the recording ends where
    // the live game did, entity for entity.
    assert_eq!(positions(app.world_mut()), walked);
}

#[test]
fn replay_reaches_same_state_when_seeked_through() {
    // Fast-forwarding is running the same ticks without presenting them, so it
    // must land on the same state as playing them one by one.
    let Recorded {
        replay,
        recorded,
        walked,
        ..
    } = record_walking_game(MovementModel::Continuous);
    let mut app = playback_app(replay);

    let seeked = ferrets_bevy_plugin::run_until_tick(app.world_mut(), recorded / 2);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert_eq!(seeked, recorded / 2, "the seek landed on its target");
    assert_eq!(report.tick, recorded + 1, "then played out to the end");
    assert_eq!(report.mismatch, None);
    assert_eq!(positions(app.world_mut()), walked);
}

//
// ─── Recorded rules ───────────────────────────────────────────────────────────
//

#[test]
fn rebuild_replays_under_recorded_movement_model() {
    // The demo's own default is Continuous, so a Cell recording is only
    // replayable if the rebuild takes the model from the header. The two models
    // are asserted to end differently first, so this cannot pass vacuously.
    let cell = record_walking_game(MovementModel::Cell);
    let continuous = record_walking_game(MovementModel::Continuous);
    assert_ne!(
        cell.walked, continuous.walked,
        "the two movement models must end differently for this to prove anything",
    );
    assert_eq!(cell.replay.header().movement_model, MovementModel::Cell);

    let mut rebuilt = playback::rebuild(cell.replay).expect("the recorded game rebuilds");
    let report = ferrets_bevy_plugin::run_playback(rebuilt.app.world_mut());

    assert_eq!(report.mismatch, None, "rebuilt under the recorded model");
    assert_eq!(positions(rebuilt.app.world_mut()), cell.walked);
}

//
// ─── Watching controls ────────────────────────────────────────────────────────
//

// The keys that steer a recording issue no commands, so they stay live while
// watching one — and they only record requests. Applying a request, and
// refusing it where it must not run (a networked session, a finished replay),
// is the engine's business, pinned by the plugin's own suites; the demo's
// contract ends at the request.

#[test]
fn pause_key_records_toggle_request() {
    let recorded = record_walking_game(MovementModel::Continuous);
    let mut app = playback_app(recorded.replay);
    app.world_mut().init_resource::<ButtonInput<KeyCode>>();
    app.world_mut().init_resource::<PauseIntent>();

    press(&mut app, KeyCode::KeyP);
    app.world_mut()
        .run_system_once(input::pause_input)
        .expect("pause input runs");
    assert_eq!(
        app.world().resource::<PauseIntent>().0,
        Some(true),
        "a running replay asks to pause",
    );

    // Once the session is paused, the same key asks for the opposite.
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);
    repress(&mut app, KeyCode::KeyP);
    app.world_mut()
        .run_system_once(input::pause_input)
        .expect("pause input runs");
    assert_eq!(app.world().resource::<PauseIntent>().0, Some(false));
}

#[test]
fn step_key_records_step_request() {
    let recorded = record_walking_game(MovementModel::Continuous);
    let mut app = playback_app(recorded.replay);
    app.world_mut().init_resource::<ButtonInput<KeyCode>>();

    press(&mut app, KeyCode::Period);
    app.world_mut()
        .run_system_once(input::step_input)
        .expect("step input runs");

    assert!(
        app.world().contains_resource::<Step>(),
        "the request is recorded for the engine to apply",
    );
}

#[test]
fn seek_key_records_target_ahead() {
    let recorded = record_walking_game(MovementModel::Continuous);
    let mut app = playback_app(recorded.replay);
    app.world_mut().init_resource::<ButtonInput<KeyCode>>();
    ferrets_bevy_plugin::run_until_tick(app.world_mut(), 10);

    press(&mut app, KeyCode::BracketRight);
    app.world_mut()
        .run_system_once(input::seek_input)
        .expect("seek input runs");

    assert_eq!(
        app.world().resource::<Seek>().0,
        10 + 10 * NOMINAL_TICK_HZ as u32,
        "ten seconds of recording ahead of the current tick",
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Ticks to play, long enough for the ordered walks to run their course.
const TICKS: u32 = 120;

/// Holds `key` down for the systems run next.
fn press(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
}

/// Releases and presses `key` again, so a second press registers as one.
fn repress(app: &mut App, key: KeyCode) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(key);
    keys.clear();
    keys.press(key);
}

/// Every live entity's position by id, the state comparison the assertions rest
/// on.
fn positions(world: &mut World) -> Vec<(SimulationId, (u64, u64))> {
    let entries = world.resource::<EntityIndex>().alive_entries();
    entries
        .into_iter()
        .filter_map(|(id, entity)| {
            let location = world.get::<LocationComponent>(entity)?;
            Some((
                id,
                (location.position.x.to_bits(), location.position.y.to_bits()),
            ))
        })
        .collect()
}

/// A recorded game and what the live run it came from did.
struct Recorded {
    replay: Replay,
    /// The last tick the recording captured.
    recorded: u32,
    /// Where the units stood before the walk.
    started: Vec<(SimulationId, (u64, u64))>,
    /// Where they stood when recording stopped.
    walked: Vec<(SimulationId, (u64, u64))>,
}

/// Plays a game in which the mission's units are ordered across the map,
/// recording it.
fn record_walking_game(model: MovementModel) -> Recorded {
    let mut app = utils::scenario_app(model);
    app.add_plugins(ReplayPlugin);
    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(
        RecordedGame::Scenario("build_army".to_string()),
        model,
        Projection::Isometric,
    );
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    replay::recorder::install_per_game(app.world_mut(), recorder);

    // Something for the recording to carry: every unit the mission placed walks
    // to one point, so the ticks hold movement, contact and arrival.
    let movers: Vec<SimulationId> = app
        .world_mut()
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for (index, id) in movers.iter().enumerate() {
        let mode = if index == 0 {
            SelectMode::Replace
        } else {
            SelectMode::Add
        };
        utils::select(&mut app, *id, mode);
    }
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::at_cell(20, 20),
            flush: true,
        },
    );

    let started = positions(app.world_mut());
    for _ in 0..TICKS {
        ferrets_bevy_plugin::run_tick(app.world_mut());
    }
    // The recorder writes the ticks that have completed, so flush the last one.
    ferrets_bevy_plugin::record_input(app.world_mut());

    let walked = positions(app.world_mut());
    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let recorded = replay.last_tick().expect("the recording holds ticks");
    Recorded {
        replay,
        recorded,
        started,
        walked,
    }
}

/// The same game rebuilt from scratch with the recording as its sole frame
/// source.
fn playback_app(replay: Replay) -> App {
    let mut app = utils::scenario_app(MovementModel::Continuous);
    app.add_plugins(ReplayPlugin);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Endless);
    ferrets_bevy_plugin::replay::playback::install_per_game(app.world_mut(), replay);
    app
}
