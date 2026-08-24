//! Watching a recording back.

use bevy::prelude::*;
use ferrets_replay::{header::ReplayHeader, replay::Replay};
use ferrets_simulation::{
    checksum,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::GameSession,
};

use crate::tick;

/// Whether an installed recording has no input for the tick the session stands
/// on — the point past which playback must not advance.
///
/// The tick loop otherwise re-derives blocked-or-running from what the input
/// queue holds, and the engine pre-seeds the lockstep warmup ticks; without this
/// a recording that covers fewer ticks than the warmup would be "played" out of
/// frames it never recorded.
pub fn replay_exhausted(playback: Option<Res<ReplayPlayback>>, session: Res<GameSession>) -> bool {
    playback.is_some_and(|playback| !playback.holds(session.tick()))
}

/// How a playback run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackReport {
    /// The tick playback stopped on.
    pub tick: u32,
    /// Whether it played every recorded tick.
    pub done: bool,
    /// The first tick whose replayed state diverged from the recording, if any.
    pub mismatch: Option<u32>,
}

/// The replay being watched.
#[derive(Resource)]
pub struct ReplayPlayback {
    replay: Replay,
    /// The final recorded tick, or `None` for a recording that holds no
    /// completed ticks at all (a crash during the very first one leaves a valid
    /// header-only file) — which must freeze before anything runs, not play a
    /// tick nobody recorded.
    last_tick: Option<u32>,
    /// Set once playback has run every recorded tick and stopped.
    done: bool,
    /// The first tick whose replayed state diverged from the recording, if any —
    /// a determinism bug surfaced by watching the replay.
    mismatch: Option<u32>,
    /// The tick last checked against the recording. Verifying one twice cannot
    /// tell us anything new, and the tick does stand still while the session is
    /// unpaused: a recorded outcome that finished the replayed session stops the
    /// counter without pausing, so without this the check would re-hash the
    /// whole world on every step, forever.
    verified: Option<u32>,
}

impl ReplayPlayback {
    /// The recording's header — the setup the recorded game was played under.
    pub fn header(&self) -> &ReplayHeader {
        self.replay.header()
    }

    /// Whether the recording holds input for `tick`.
    ///
    /// A tick the recording covers may legitimately hold no commands — every
    /// player was idle — so emptiness cannot answer this; the recorded range is
    /// what does.
    pub fn holds(&self, tick: u32) -> bool {
        self.last_tick.is_some_and(|last| tick <= last)
    }

    /// Whether the recording has been played out: every tick it holds has run,
    /// and there is no input for the tick the session now stands on. The
    /// session is [`Blocked`](ferrets_simulation::session::SessionState::Blocked)
    /// from that point — not paused, which is a player's choice — so nothing
    /// advances until a new game replaces it.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The first tick that failed checksum verification, if playback diverged.
    pub fn mismatch(&self) -> Option<u32> {
        self.mismatch
    }
}

/// Installs `replay` for playback, making it the sole frame source. Call at game
/// start with a [`Replay`] read from the caller's input stream; the session must
/// already be configured to match its header.
pub fn install_per_game(world: &mut World, replay: Replay) {
    let last_tick = replay.last_tick();
    world.insert_resource(ReplayPlayback {
        replay,
        last_tick,
        done: false,
        mismatch: None,
        verified: None,
    });
}

/// Removes the playback when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources): left installed,
/// it would keep supplying the recorded frames into the next game.
pub(crate) fn remove_per_game(world: &mut World) {
    world.remove_resource::<ReplayPlayback>();
}

/// Replays every remaining recorded tick as fast as the machine manages,
/// returning what happened. A recording that cannot be played to its end — a
/// diverged one whose session ends early, say — reports `done: false` rather
/// than spinning.
///
/// Panics if no playback is installed.
pub fn run_playback(world: &mut World) -> PlaybackReport {
    let tick = tick::run_until_tick(world, u32::MAX);
    let playback = world.resource::<ReplayPlayback>();
    PlaybackReport {
        tick,
        done: playback.is_done(),
        mismatch: playback.mismatch(),
    }
}

/// The replay frame source during playback: supplies every live slot's recorded
/// frame (or idle) `SYNC_LATENCY` ticks ahead, re-applies the drops the recording
/// saw, and freezes the session once the final recorded tick has been played.
pub fn supply_replay_input(
    mut playback: ResMut<ReplayPlayback>,
    mut frames: ResMut<InputFrames>,
    mut session: ResMut<GameSession>,
) {
    let now = session.tick();
    // No input for the tick the session stands on: the recording is played out
    // (or held nothing to begin with). Say so, and block — the frame source
    // cannot fill this tick, which is exactly what blocking means everywhere
    // else. A session the recorded outcome already finished stops on its own.
    if !playback.holds(now) {
        if !playback.done {
            playback.done = true;
            if session.result().is_none() {
                session.set_blocked(true);
            }
        }
        return;
    }
    // Finished before the recording's end: a divergence ended the replayed
    // session early. Nothing it sources would ever be consumed, and `done`
    // stays false — the recording was not played out.
    if session.result().is_some() {
        return;
    }

    // Re-apply the drops that took effect at this tick before sourcing input, so
    // the session's required set and the victory check exclude a dropped player
    // exactly as the live game did — not replay it as idle but present.
    for &player in playback.replay.drops_at(now) {
        if !session.is_player_dropped(player) {
            session.drop_player(player, now);
        }
    }

    let target = now + SYNC_LATENCY;
    // Only ticks the recording covers may be sourced. Past its end `inputs_at`
    // answers "no commands", which is indistinguishable from a tick where every
    // player was idle — sourcing it would fabricate input the recording never
    // held and replay ticks that never happened.
    if !playback.holds(target) {
        return;
    }
    let recorded = playback.replay.inputs_at(target);
    for slot in session.slots() {
        let player = slot.id();
        // A player who is out is required for no tick from its drop or
        // elimination on, so it has no recorded frame and needs none. (A drop
        // is re-applied from the recording above; an elimination the replayed
        // simulation re-derives on its own.)
        if session.is_player_out(player) {
            continue;
        }
        let frame = match recorded.iter().find(|(recorded, _)| *recorded == player) {
            Some((_, commands)) => PlayerFrame {
                player,
                tick: target,
                commands: commands.clone(),
            },
            None => PlayerFrame::idle(player, target),
        };
        frames.push_frame(frame);
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
    // Verifying a tick twice cannot say anything new, and the tick does stand
    // still while this system keeps running: a recorded outcome that finished the
    // replayed session stops the counter without pausing. The condition that
    // would answer "did this step advance the tick" is the single run-condition
    // evaluation on the simulation chain's group — an optional plugin cannot
    // join that group without inverting the dependency, so this system is made
    // idempotent instead, the same way `record_input` carries `next_tick`.
    if playback.verified == Some(completed) {
        return;
    }
    let Some(expected) = playback.replay.checksum_at(completed) else {
        return;
    };
    let actual = checksum::state_checksum(world);
    world.resource_mut::<ReplayPlayback>().verified = Some(completed);
    if actual != expected {
        eprintln!(
            "replay desync at tick {completed}: recorded checksum {expected:#x}, replayed {actual:#x}",
        );
        world.resource_mut::<ReplayPlayback>().mismatch = Some(completed);
    }
}
