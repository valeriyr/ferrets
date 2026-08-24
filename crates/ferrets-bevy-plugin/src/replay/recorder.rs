//! Recording the running game.

use bevy::prelude::*;
use ferrets_replay::{record::TickRecord, recorder::Recorder};
use ferrets_simulation::{
    checksum::{self, CHECKSUM_INTERVAL},
    command::PlayerCommand,
    input::InputFrames,
    session::{GameSession, player_slot::PlayerId},
};

/// The active recording.
///
/// A `NonSend` resource because [`Recorder`]'s writer need only be `Send`. Absent
/// for a game that is not being recorded (e.g. while watching a replay).
pub struct ReplayRecorder {
    recorder: Recorder,
    next_tick: u32,
}

/// Installs `recorder` so the running game is recorded. Call at game start with
/// a [`Recorder`] opened over the caller's output stream.
pub fn install_per_game(world: &mut World, recorder: Recorder) {
    world.insert_non_send_resource(ReplayRecorder {
        recorder,
        next_tick: 0,
    });
}

/// Removes the recorder when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources): left installed,
/// it would keep writing the next game into the last one's file.
pub(crate) fn remove_per_game(world: &mut World) {
    world.remove_non_send_resource::<ReplayRecorder>();
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
        let inputs = realized_inputs(world, tick);
        let dropped = dropped_at(world, tick);
        // The world holds the state after the most-recently-completed tick, so a
        // checksum is only valid for that one.
        let checksum = (tick == current - 1 && tick.is_multiple_of(CHECKSUM_INTERVAL))
            .then(|| checksum::state_checksum(world));
        let record = TickRecord {
            tick,
            inputs,
            dropped,
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

/// The per-player commands of a completed tick, idle players omitted. Reads
/// the players the tick required *at execution* — a player dropped since still
/// contributes to the ticks it was live for, and a dropped player's unexecuted
/// leftovers never enter the recording.
fn realized_inputs(world: &World, tick: u32) -> Vec<(PlayerId, Vec<PlayerCommand>)> {
    let required = world.resource::<GameSession>().required_players(tick);
    let Some(ready) = world
        .resource::<InputFrames>()
        .ready_commands(tick, &required)
    else {
        return Vec::new();
    };
    ready
        .into_iter()
        .filter(|(_, commands)| !commands.is_empty())
        .map(|(player, commands)| (player, commands.to_vec()))
        .collect()
}

/// The players whose drop took effect exactly at `tick` — those playback must
/// re-apply there so the dropped set (and the outcome that turns on it) tracks
/// the recorded game.
fn dropped_at(world: &World, tick: u32) -> Vec<PlayerId> {
    let session = world.resource::<GameSession>();
    session
        .dropped_players()
        .filter(|&player| session.drop_tick(player) == Some(tick))
        .collect()
}
