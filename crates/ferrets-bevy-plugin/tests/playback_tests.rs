//! Driving playback and the session directly, without a frame loop.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::{NetworkActive, PauseIntent, ReplayPlayback, ReplayPlugin, Seek, Step};
use ferrets_content::{
    entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_replay::{buffer::SharedBuffer, recorder::Recorder, replay::Replay};
use ferrets_simulation::{
    command::PlayerCommand,
    input::{InputFrames, PlayerFrame},
    session::{GameSession, finish_policy::FinishPolicy},
};

use utils::{GROUND, GROUND_LAYER};

//
// ─── Running a recording ──────────────────────────────────────────────────────
//

#[test]
fn run_playback_reaches_recorded_end_without_mismatch() {
    let (replay, recorded) = recorded_game(40);
    let mut app = playback_app(replay);

    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert!(report.done, "played every recorded tick");
    assert_eq!(report.mismatch, None, "verified every recorded checksum");
    // The tick after the last recorded one is where playback notices the end and
    // freezes, so that is the tick it stops on.
    assert_eq!(report.tick, recorded + 1);
}

#[test]
fn run_playback_reports_spawn_recording_carried() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);

    ferrets_bevy_plugin::run_playback(app.world_mut());

    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);
}

#[test]
fn run_playback_reports_finished_game_played_out() {
    // A recorded game that ended with a result finishes its replayed session the
    // same way — no frozen final state to pause on — and playback still counts
    // as done: every recorded tick ran and verified.
    let mut recorded = lone_base_app();
    let buffer = utils::record_into(
        &mut recorded,
        &utils::skirmish_header(utils::human_slots(2), FinishPolicy::LastStanding),
    );
    for _ in 0..8 {
        // The building-less player has no frame source of its own; idle frames
        // stand in for it, exactly as they would come off a real peer's wire.
        let tick = utils::tick(&recorded);
        recorded
            .world_mut()
            .resource_mut::<InputFrames>()
            .push_frame(PlayerFrame::idle(1, tick));
        ferrets_bevy_plugin::run_tick(recorded.world_mut());
    }
    assert!(
        recorded
            .world()
            .resource::<GameSession>()
            .result()
            .is_some(),
        "the lone building-holding side won",
    );
    ferrets_bevy_plugin::record_input(recorded.world_mut());
    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let last = replay.last_tick().expect("the recording holds ticks");

    let mut app = lone_base_app();
    ferrets_bevy_plugin::replay::playback::install_per_game(app.world_mut(), replay);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert!(report.done, "played every recorded tick");
    assert_eq!(report.mismatch, None);
    assert_eq!(report.tick, last + 1);
    let session = app.world().resource::<GameSession>();
    assert!(
        session.result().is_some(),
        "the replayed session re-derived the recorded outcome",
    );
    // Why `ReplayPlayback::verified` has to exist: a session the recorded
    // outcome finished is NOT paused, so the `FixedLast` checks keep running on
    // a tick that no longer advances. They cannot be gated off instead — the
    // final tick's own verification happens in the very step the session stops,
    // when it is already inactive and already out of recording.
    assert!(!session.is_paused(), "finished, not paused");
    assert!(!session.is_advancing(), "and not advancing");
}

#[test]
fn recording_holding_no_ticks_plays_nothing() {
    // A crash during the very first tick leaves a valid header-only file; played
    // back, it freezes before anything runs — a tick nobody recorded must not
    // execute, and must not read as verified.
    let buffer = SharedBuffer::default();
    let _recorder = Recorder::new(
        buffer.clone(),
        &utils::skirmish_header(utils::human_slots(1), FinishPolicy::Endless),
    )
    .expect("start recording");
    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    assert_eq!(replay.last_tick(), None, "nothing was recorded");

    let mut app = playback_app(replay);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert_eq!(report.tick, 0, "no unrecorded tick executed");
    assert!(report.done, "all zero recorded ticks played");
    assert_eq!(report.mismatch, None);
}

//
// ─── Seeking ──────────────────────────────────────────────────────────────────
//

#[test]
fn run_until_tick_lands_on_target() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);

    let reached = ferrets_bevy_plugin::run_until_tick(app.world_mut(), 20);

    assert_eq!(reached, 20);
    assert_eq!(utils::tick(&app), 20);
}

#[test]
fn run_until_tick_stops_at_recorded_end() {
    // Asking for more than was recorded stops where the recording does, rather
    // than spinning on a frozen tick.
    let (replay, recorded) = recorded_game(40);
    let mut app = playback_app(replay);

    let reached = ferrets_bevy_plugin::run_until_tick(app.world_mut(), recorded + 500);

    assert_eq!(reached, recorded + 1, "stopped where the recording ends");
    assert!(app.world().resource::<ReplayPlayback>().is_done());
}

#[test]
fn seek_resource_fast_forwards_playback() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    app.world_mut().insert_resource(Seek(25));

    apply_seek_fully(&mut app);

    assert_eq!(utils::tick(&app), 25);
}

#[test]
fn seek_leaves_paused_playback_paused() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);
    app.world_mut().insert_resource(Seek(25));

    apply_seek_fully(&mut app);

    assert_eq!(
        utils::tick(&app),
        25,
        "seeking runs its ticks even while paused"
    );
    assert!(app.world().resource::<GameSession>().is_paused());
}

#[test]
fn seek_fast_forwards_live_local_game() {
    // Off the network a seek needs no recording: a live game's own simulation is
    // fast-forwarded the same way, its frame sources feeding every tick.
    let mut app = base_app();

    app.world_mut().insert_resource(Seek(25));
    apply_seek_fully(&mut app);

    assert_eq!(utils::tick(&app), 25);
    assert!(
        !app.world().resource::<GameSession>().is_paused(),
        "a game seeked while running keeps running",
    );
}

#[test]
fn played_out_replay_blocks_and_cannot_be_resumed() {
    // A played-out recording leaves the session BLOCKED — the frame source has
    // no input for the tick, which is what blocking means — not paused, which is
    // a player's choice. So a resume changes nothing on its own: there is no
    // pause to lift and nothing to advance into. No control path needs to ask
    // the replay whether it is finished.
    let (replay, recorded) = recorded_game(40);
    let mut app = playback_app(replay);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    assert!(report.done, "the recording played out");
    assert_eq!(
        report.tick,
        recorded + 1,
        "stopped on the first tick it lacks"
    );
    let session = app.world().resource::<GameSession>();
    assert!(session.is_blocked(), "blocked, for want of input");
    assert!(!session.is_paused(), "and not pretending to be paused");

    app.world_mut().resource_mut::<PauseIntent>().0 = Some(false);
    app.world_mut()
        .run_system_once(ferrets_bevy_plugin::apply_local_pause)
        .expect("pause intent applies");
    ferrets_bevy_plugin::run_tick(app.world_mut());

    assert_eq!(
        utils::tick(&app),
        recorded + 1,
        "still nothing to advance into"
    );
    assert!(app.world().resource::<GameSession>().is_blocked());
}

#[test]
fn seek_on_networked_session_is_discarded() {
    // Running ahead of the fixed loop would leave a networked node ticks past
    // its peers; the engine refuses the request so no frontend can get it wrong.
    let mut app = base_app();
    app.world_mut().insert_resource(NetworkActive);
    let before = utils::tick(&app);

    app.world_mut().insert_resource(Seek(before + 25));
    apply_seek_fully(&mut app);

    assert_eq!(utils::tick(&app), before, "nothing advanced");
}

#[test]
fn step_request_on_networked_session_is_discarded() {
    let mut app = base_app();
    app.world_mut().insert_resource(NetworkActive);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);
    let before = utils::tick(&app);

    app.world_mut().insert_resource(Step);
    ferrets_bevy_plugin::apply_step(app.world_mut());

    assert_eq!(utils::tick(&app), before, "nothing advanced");
    assert!(!app.world().contains_resource::<Step>());
}

#[test]
fn seek_giving_up_mid_flight_restores_pause_state() {
    // A seek in flight holds the session paused between its frames, whichever
    // state it found. If it becomes ineligible before reaching its target it must
    // put back the state it began under — both ways round, which is also what
    // says the state is remembered rather than assumed: a game the player was
    // watching at speed must not be left frozen by machinery that gave up, and
    // one they had paused must not be handed back running.
    for (paused_before, case) in [(false, "a running game"), (true, "a paused game")] {
        // A live local game, which advances for as long as it is asked to, and a
        // target no single frame budget could reach on any machine — so the seek
        // is certainly still in flight, whatever a tick costs here.
        let mut app = base_app();
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_paused(paused_before);
        app.world_mut().insert_resource(Seek(u32::MAX));
        ferrets_bevy_plugin::apply_seek(app.world_mut());
        assert!(
            app.world().contains_resource::<Seek>(),
            "{case}: the seek carried over to another frame",
        );
        assert!(
            app.world().resource::<GameSession>().is_paused(),
            "{case}: held paused between them",
        );

        // Now the session cannot advance, which is one of the reasons a seek is
        // refused.
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_blocked(true);
        ferrets_bevy_plugin::apply_seek(app.world_mut());

        assert!(
            !app.world().contains_resource::<Seek>(),
            "{case}: the refused seek is consumed",
        );
        assert_eq!(
            app.world().resource::<GameSession>().is_paused(),
            paused_before,
            "{case}: the pause state the seek began under is back",
        );
    }
}

#[test]
fn seek_on_finished_replay_is_discarded() {
    // Past the recorded end there is nothing left to seek through — running on
    // would simulate ticks nobody recorded, past the "replay ended" freeze.
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    app.world_mut().insert_resource(Seek(report.tick + 100));
    apply_seek_fully(&mut app);

    assert_eq!(utils::tick(&app), report.tick, "nothing advanced");
    assert!(
        app.world().resource::<GameSession>().is_blocked(),
        "a played-out recording holds the session blocked",
    );
}

//
// ─── Stepping while paused ────────────────────────────────────────────────────
//

#[test]
fn step_while_paused_advances_exactly_one_tick() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    ferrets_bevy_plugin::run_until_tick(app.world_mut(), 10);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);

    ferrets_bevy_plugin::run_tick_while_paused(app.world_mut());

    assert_eq!(utils::tick(&app), 11);
    assert!(
        app.world().resource::<GameSession>().is_paused(),
        "still paused afterwards"
    );
}

#[test]
fn step_request_advances_exactly_one_tick_while_paused() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    ferrets_bevy_plugin::run_until_tick(app.world_mut(), 10);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);

    app.world_mut().insert_resource(Step);
    ferrets_bevy_plugin::apply_step(app.world_mut());

    assert_eq!(utils::tick(&app), 11);
    assert!(
        !app.world().contains_resource::<Step>(),
        "the request is consumed"
    );
    assert!(app.world().resource::<GameSession>().is_paused());
}

#[test]
fn step_request_while_unpaused_is_consumed_without_advancing() {
    // Stepping is for walking a paused moment; a running game already advances
    // on its own, so a stray request does nothing but disappear.
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    ferrets_bevy_plugin::run_until_tick(app.world_mut(), 10);

    app.world_mut().insert_resource(Step);
    ferrets_bevy_plugin::apply_step(app.world_mut());

    assert_eq!(utils::tick(&app), 10);
    assert!(!app.world().contains_resource::<Step>());
}

#[test]
fn step_request_on_finished_replay_is_discarded() {
    // Same rule as a seek: past the recorded end there is nothing to step into.
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    let report = ferrets_bevy_plugin::run_playback(app.world_mut());

    app.world_mut().insert_resource(Step);
    ferrets_bevy_plugin::apply_step(app.world_mut());

    assert_eq!(utils::tick(&app), report.tick, "nothing advanced");
    assert!(!app.world().contains_resource::<Step>());
}

#[test]
fn paused_playback_does_not_advance_on_its_own() {
    let (replay, _) = recorded_game(40);
    let mut app = playback_app(replay);
    ferrets_bevy_plugin::run_until_tick(app.world_mut(), 10);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_paused(true);

    ferrets_bevy_plugin::run_tick(app.world_mut());

    assert_eq!(utils::tick(&app), 10);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Plays `ticks` ticks of a one-player game that spawns a soldier early on,
/// recording it, and returns the recording with the last tick it captured.
fn recorded_game(ticks: u32) -> (Replay, u32) {
    let mut app = base_app();
    let buffer = utils::record_into(
        &mut app,
        &utils::skirmish_header(utils::human_slots(1), FinishPolicy::Endless),
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::pos(10, 10),
        },
    );
    for _ in 0..ticks {
        ferrets_bevy_plugin::run_tick(app.world_mut());
    }
    // The recorder writes the ticks that have completed, so flush the last one.
    ferrets_bevy_plugin::record_input(app.world_mut());

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let last = replay.last_tick().expect("the recording holds ticks");
    (replay, last)
}

fn playback_app(replay: Replay) -> App {
    let mut app = base_app();
    ferrets_bevy_plugin::replay::playback::install_per_game(app.world_mut(), replay);
    app
}

/// Applies a pending seek to completion, however many frame budgets it takes —
/// the seek deliberately spreads a long fast-forward over frames, which a test
/// drains in one go.
fn apply_seek_fully(app: &mut App) {
    while app.world().contains_resource::<Seek>() {
        ferrets_bevy_plugin::apply_seek(app.world_mut());
    }
}

/// A started two-player `LastStanding` app where only player 0 holds a building
/// (spawned before the recording begins, as a scene spawner would), so the game
/// ends the moment the victory check first runs.
fn lone_base_app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    app.add_plugins(ReplayPlugin);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(
            EntityTypeDef::new("base")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(30)
                .with_tags(["building"]),
        );
        registry.validate();
    }
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    utils::spawn_owned(&mut app, "base", 5, 5, 0);
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

fn base_app() -> App {
    let mut app = utils::make_app(utils::human_slots(1));
    app.add_plugins(ReplayPlugin);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30),
        );
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
