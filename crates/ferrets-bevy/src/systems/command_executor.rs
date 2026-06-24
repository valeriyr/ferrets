use bevy::prelude::*;
use ferrets_simulation::{game_loop, session::GameSession};

/// Executes the player commands scheduled for the current tick, or blocks the
/// session when the frame is not yet complete (waiting on a peer/AI).
///
/// Runs while the session is merely *active* (running or blocked) so it can
/// detect when a previously-missing frame becomes ready and resume.
pub fn command_executor(world: &mut World) {
    let current_tick = world.resource::<GameSession>().tick();
    let ready = game_loop::executor::tick(world, current_tick);
    world.resource_mut::<GameSession>().set_blocked(!ready);
}
