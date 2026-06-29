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
/// Net-agnostic: it only writes the input queue.
pub fn flush_input(
    mut pending: ResMut<PendingInput>,
    mut frames: ResMut<InputFrames>,
    session: Res<GameSession>,
) {
    let commands: Vec<PlayerCommand> = pending.commands.drain(..).collect();
    frames.push_frame(PlayerFrame {
        player: session.local_player(),
        tick: session.tick() + SYNC_LATENCY,
        commands,
    });
}
