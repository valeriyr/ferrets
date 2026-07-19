//! The skirmish the demo is currently playing.

use bevy::prelude::*;
use ferrets_simulation::skirmish::Skirmish;

/// The skirmish the current game runs: the session is configured from it and a
/// recording embeds it. Absent outside a skirmish game.
#[derive(Resource)]
pub struct CurrentSkirmish(pub Skirmish);
