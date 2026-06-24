//! Top-level game flow: the race-select menu, then the running game.

use bevy::prelude::*;

/// Which screen the demo is showing.
#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    /// Race-select menu, shown before the game starts.
    #[default]
    Menu,
    /// The game is running.
    InGame,
}

/// The race the local player picked in the menu (a registered race id, e.g.
/// `"human"`).
#[derive(Resource, Default)]
pub struct ChosenRace(pub String);
