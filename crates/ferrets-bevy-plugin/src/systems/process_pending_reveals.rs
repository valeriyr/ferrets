use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Retries reappearing every entity that finished an order while boxed-in and is
/// still waiting for a free cell to return to the map.
pub fn process_pending_reveals(world: &mut World) {
    game_loop::pending_reveal::process_pending_reveals(world);
}
