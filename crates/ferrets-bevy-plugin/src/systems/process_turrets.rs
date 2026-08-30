use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Works every turret a body carries (see
/// [`game_loop::turret::process_turrets`]).
pub fn process_turrets(world: &mut World) {
    game_loop::turret::process_turrets(world);
}
