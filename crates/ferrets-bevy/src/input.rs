//! Local player commands buffered between `Update` and `FixedUpdate`.
//!
//! Push commands from any `Update` system (keyboard, mouse). The [`flush_input`](super::systems::flush_input)
//! system drains them into `InputFrames` at the start of each `FixedUpdate` tick.

use bevy::prelude::*;
use ferrets_simulation::command::PlayerCommand;

#[derive(Resource, Default)]
pub struct PendingInput {
    pub(super) commands: Vec<PlayerCommand>,
}

impl PendingInput {
    pub fn push(&mut self, command: PlayerCommand) {
        self.commands.push(command);
    }
}
