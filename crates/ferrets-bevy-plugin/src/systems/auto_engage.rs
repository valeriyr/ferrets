use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Engages idle entities per their stance (see
/// [`game_loop::auto_engage::tick`]).
pub fn auto_engage(world: &mut World) {
    game_loop::auto_engage::tick(world);
}
