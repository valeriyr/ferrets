//! Recording every game and watching one back.
//!
//! Recording captures the realized input for each completed tick (regardless of
//! whether it came from the local player, an AI, or the network) and streams it to
//! a [`Recorder`] the caller supplies. Playback installs a loaded [`Replay`] and
//! makes it the sole frame source for every slot, feeding the recorded frames back
//! through the normal lockstep path; the simulation, being deterministic, retraces
//! the original game. Each recorded checksum is recomputed on playback to verify
//! that determinism still holds.
//!
//! The caller owns file IO: build the [`Recorder`]/[`Replay`] over its own streams
//! and install them with [`install_replay_recorder`]/[`install_replay_playback`].

use bevy::prelude::*;
use ferrets_replay::record::TickRecord;
use ferrets_replay::recorder::Recorder;
use ferrets_replay::replay::Replay;
use ferrets_simulation::{
    checksum::{CHECKSUM_INTERVAL, state_checksum},
    command::PlayerCommand,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::{GameSession, player_slot::PlayerId},
};

use crate::{SimulationSet, session_is_active, session_is_not_paused, session_is_running, systems};

/// The active recording.
///
/// A `NonSend` resource because [`Recorder`]'s writer need only be `Send`. Absent
/// for a game that is not being recorded (e.g. while watching a replay).
pub struct ReplayRecorder {
    recorder: Recorder,
    next_tick: u32,
}

/// The replay being watched.
#[derive(Resource)]
pub struct ReplayPlayback {
    replay: Replay,
    last_tick: u32,
    /// Set once playback has run past the final recorded tick and frozen.
    done: bool,
    /// The first tick whose replayed state diverged from the recording, if any —
    /// a determinism bug surfaced by watching the replay.
    mismatch: Option<u32>,
}

impl ReplayPlayback {
    /// Whether playback has reached the end and frozen on the final state.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The first tick that failed checksum verification, if playback diverged.
    pub fn mismatch(&self) -> Option<u32> {
        self.mismatch
    }
}

/// Installs a recorder so the running game is recorded. Call at game start with a
/// [`Recorder`] opened over the caller's output stream.
pub fn install_replay_recorder(world: &mut World, recorder: Recorder) {
    world.insert_non_send_resource(ReplayRecorder {
        recorder,
        next_tick: 0,
    });
}

/// Installs a replay for playback, making it the sole frame source. Call at game
/// start with a [`Replay`] read from the caller's input stream; the session must
/// already be configured to match its header.
pub fn install_replay_playback(world: &mut World, replay: Replay) {
    let last_tick = replay.last_tick().unwrap_or(0);
    world.insert_resource(ReplayPlayback {
        replay,
        last_tick,
        done: false,
        mismatch: None,
    });
}

/// Records the game and replays one back.
///
/// Requires [`SimulationPlugin`](crate::SimulationPlugin). The systems run only
/// once a recorder or playback is installed, so this plugin is safe to add for
/// every game.
#[derive(Default)]
pub struct ReplayPlugin;

impl Plugin for ReplayPlugin {
    fn build(&self, app: &mut App) {
        // The replay's frames must reach the input queue before the executor
        // consumes the tick, so this runs before flush_input like the other frame
        // sources. It is the only source during playback (the live sources are
        // gated off), so it fills every slot.
        app.add_systems(
            FixedUpdate,
            supply_replay_input
                .in_set(SimulationSet)
                .before(systems::flush_input)
                .run_if(
                    session_is_active
                        .and(session_is_not_paused)
                        .and(resource_exists::<ReplayPlayback>),
                ),
        );
        // After the tick has advanced: record the tick just completed (recorder
        // present) and/or verify its checksum against the recording (playback
        // present). Both read the post-tick world, so they share an anchor and a
        // recorded checksum compares like-for-like.
        app.add_systems(
            FixedLast,
            (record_input, verify_replay_checksum)
                .chain()
                .run_if(session_is_running.and(session_is_not_paused)),
        );
    }
}

/// The replay frame source during playback: supplies every slot's recorded frame
/// (or idle) `SYNC_LATENCY` ticks ahead, and freezes the session once the final
/// recorded tick has been played.
pub fn supply_replay_input(
    mut playback: ResMut<ReplayPlayback>,
    mut frames: ResMut<InputFrames>,
    mut session: ResMut<GameSession>,
) {
    let now = session.tick();
    if now > playback.last_tick {
        // Past the end: freeze on the final state so the result stays on screen.
        if !playback.done {
            session.set_paused(true);
            playback.done = true;
        }
        return;
    }

    let target = now + SYNC_LATENCY;
    let recorded = playback.replay.inputs_at(target);
    for slot in session.slots() {
        let player = slot.id();
        match recorded.iter().find(|(recorded, _)| *recorded == player) {
            Some((_, commands)) => frames.push_frame(PlayerFrame {
                player,
                tick: target,
                commands: commands.clone(),
            }),
            None => frames.push_frame(PlayerFrame::idle(player, target)),
        }
    }
}

/// Records every tick completed since the last call, attaching the state checksum
/// to the most-recently-completed tick at the checksum interval. Batching keeps it
/// correct across a blocked tick (which advances nothing until its frame arrives).
pub fn record_input(world: &mut World) {
    if world.get_non_send_resource::<ReplayRecorder>().is_none() {
        return;
    }
    // Ticks below the current one have executed; their input is final.
    let current = world.resource::<GameSession>().tick();
    loop {
        let tick = world.non_send_resource::<ReplayRecorder>().next_tick;
        if tick >= current {
            break;
        }
        let inputs = realized_inputs(world.resource::<InputFrames>(), tick);
        // The world holds the state after the most-recently-completed tick, so a
        // checksum is only valid for that one.
        let checksum = (tick == current - 1 && tick.is_multiple_of(CHECKSUM_INTERVAL))
            .then(|| state_checksum(world));
        let record = TickRecord {
            tick,
            inputs,
            checksum,
        };
        let mut recorder = world.non_send_resource_mut::<ReplayRecorder>();
        if let Err(error) = recorder.recorder.record(&record) {
            eprintln!("failed to record replay tick {tick}: {error}");
            break;
        }
        recorder.next_tick = tick + 1;
    }
}

/// Recomputes the checksum of the tick just completed and compares it to the
/// recording, flagging the first divergence — a determinism regression caught by
/// watching the replay.
pub fn verify_replay_checksum(world: &mut World) {
    let Some(playback) = world.get_resource::<ReplayPlayback>() else {
        return;
    };
    if playback.mismatch.is_some() {
        return;
    }
    let current = world.resource::<GameSession>().tick();
    let Some(completed) = current.checked_sub(1) else {
        return;
    };
    let Some(expected) = playback.replay.checksum_at(completed) else {
        return;
    };
    let actual = state_checksum(world);
    if actual != expected {
        eprintln!(
            "replay desync at tick {completed}: recorded checksum {expected:#x}, replayed {actual:#x}",
        );
        world.resource_mut::<ReplayPlayback>().mismatch = Some(completed);
    }
}

/// The per-player commands of a completed tick, idle players omitted.
fn realized_inputs(frames: &InputFrames, tick: u32) -> Vec<(PlayerId, Vec<PlayerCommand>)> {
    let Some(frame) = frames.get_ready(tick) else {
        return Vec::new();
    };
    frame
        .iter()
        .filter(|(_, commands)| !commands.is_empty())
        .map(|(player, commands)| (player, commands.to_vec()))
        .collect()
}
