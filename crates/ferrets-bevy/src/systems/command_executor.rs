use bevy::prelude::*;
use ferrets_simulation::{game_loop, session::GameSession};

/// Executes the player commands scheduled for the current tick.
pub fn command_executor(world: &mut World) {
    let current_tick = world.resource::<GameSession>().tick();
    game_loop::executor::tick(world, current_tick);
}
