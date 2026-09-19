use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages every timed life by one tick, ending the ones whose time is up.
pub fn process_lifetimes(world: &mut World) {
    game_loop::stats::process_lifetimes(world);
}
