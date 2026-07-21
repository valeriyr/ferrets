use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Sends fleeing-stance entities running from fresh hits (see
/// [`game_loop::flee::tick`]).
pub fn flee(world: &mut World) {
    game_loop::flee::tick(world);
}
