//! Recording every game and watching one back.
//!
//! Recording captures the realized input for each completed tick (regardless of
//! whether it came from the local player, an AI, or the network) and streams it to
//! a [`Recorder`](ferrets_replay::recorder::Recorder) the caller supplies.
//! Playback installs a loaded [`Replay`](ferrets_replay::replay::Replay) and
//! makes it the sole frame source for every slot, feeding the recorded frames back
//! through the normal lockstep path; the simulation, being deterministic, retraces
//! the original game. Each recorded checksum is recomputed on playback to verify
//! that determinism still holds.
//!
//! The caller owns file IO: build the recorder or the replay over its own streams
//! and install it with its module's `install_per_game`.

pub mod playback;
pub mod recorder;

use bevy::prelude::*;

use crate::{FixedLastSet, FixedUpdateSet, session_is_not_paused};

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
        // Gated only on not-paused, not on `active`: the session a recorded
        // outcome finishes stops being active, and the end-of-recording check
        // must still run there to mark playback done.
        app.add_systems(
            FixedUpdate,
            playback::supply_replay_input
                .in_set(FixedUpdateSet::Sources)
                .run_if(session_is_not_paused.and(resource_exists::<playback::ReplayPlayback>)),
        );
        // After the tick has advanced: record the tick just completed (recorder
        // present) and/or verify its checksum against the recording (playback
        // present). Both read the post-tick world, so they share an anchor and a
        // recorded checksum compares like-for-like.
        //
        // Gated only on not-paused, not on `running`: the tick whose result ends
        // the game (a drop or a last-standing win) has already been executed and
        // its counter advanced, but `check_game_result` moved the session to
        // Finished before this runs. Recording only while running would drop that
        // final tick — and the outcome that rides on it — from the replay. Both
        // systems catch up to the current tick and then idle, so running past the
        // finish is a bounded no-op.
        // Work belonging to the tick just executed, so it lands in the step's
        // `Work` phase — which the cadence measurement closes after, making the
        // recording and checksum cost part of what the tick cost.
        app.add_systems(
            FixedLast,
            (recorder::record_input, playback::verify_replay_checksum)
                .chain()
                .in_set(FixedLastSet::Work)
                .run_if(session_is_not_paused),
        );
    }
}
