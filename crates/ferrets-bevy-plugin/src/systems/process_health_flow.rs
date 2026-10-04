use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Moves each health pool by one tick's regeneration and drain.
pub fn process_health_flow(world: &mut World) {
    game_loop::flows::process_health_flow(world);
}
