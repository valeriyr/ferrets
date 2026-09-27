use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Records how long each entity's order queue has stood empty.
pub fn note_idleness(world: &mut World) {
    game_loop::orders::note_idleness(world);
}
