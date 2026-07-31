use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Refills each health pool by one tick's regeneration.
pub fn process_health_regen(world: &mut World) {
    game_loop::stats::process_health_regen(world);
}
