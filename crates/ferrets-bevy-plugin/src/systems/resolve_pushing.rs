use bevy::prelude::*;
use ferrets_simulation::game_loop;

pub fn resolve_pushing(world: &mut World) {
    game_loop::pushing::resolve(world);
}
