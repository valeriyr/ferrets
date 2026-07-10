use bevy::prelude::*;
use ferrets_simulation::{
    command::PlayerCommand,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::GameSession,
};

use crate::input::PendingInput;

/// The local player's frame source: drains pending commands into a frame
/// scheduled `SYNC_LATENCY` ticks ahead (so peers have time to receive it) and
/// records it in the input queue. An empty command list is still a real frame —
/// "the local player did nothing this tick".
///
/// Flushes at most once per tick: while the session is blocked this system
/// reruns with the tick frozen, and the target's frame is already committed
/// and immutable — commands issued during the freeze stay buffered in
/// [`PendingInput`] until the tick advances and a fresh target opens.
///
/// Net-agnostic: it only writes the input queue.
pub fn flush_input(
    mut pending: ResMut<PendingInput>,
    mut frames: ResMut<InputFrames>,
    session: Res<GameSession>,
) {
    let player = session.local_player();
    let target = session.tick() + SYNC_LATENCY;
    if frames.has_frame(player, target) {
        return;
    }
    let commands: Vec<PlayerCommand> = pending.commands.drain(..).collect();
    frames.push_frame(PlayerFrame {
        player,
        tick: target,
        commands,
    });
}
