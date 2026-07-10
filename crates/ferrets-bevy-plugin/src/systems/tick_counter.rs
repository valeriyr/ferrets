use bevy::prelude::*;
use ferrets_simulation::{game_loop, session::GameSession};

pub fn tick_counter(mut session: ResMut<GameSession>) {
    game_loop::tick_counter::tick(&mut session);
}
