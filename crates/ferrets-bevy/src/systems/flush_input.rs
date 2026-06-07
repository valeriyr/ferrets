use bevy::prelude::*;
use ferrets_simulation::{input::InputFrames, session::GameSession};

use crate::input::PendingInput;

pub fn flush_input(
    mut pending: ResMut<PendingInput>,
    mut frames: ResMut<InputFrames>,
    session: Res<GameSession>,
) {
    frames.push_local(
        session.local_player(),
        session.tick(),
        pending.commands.drain(..),
    );
}
