use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Refits the buffs held on a requirement: ends the lapsed, applies the
/// passives whose requirement each entity meets.
pub fn refit_held_buffs(world: &mut World) {
    game_loop::buffs::refit_held_buffs(world);
}
