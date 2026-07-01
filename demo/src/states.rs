//! The demo's top-level flow, modelled as Bevy states.

use bevy::prelude::*;

/// Which screen the demo is showing.
#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    /// The main menu: pick local, host, or join.
    #[default]
    Menu,
    /// The lobby: configure slots and start the game.
    Lobby,
    /// The game is running.
    InGame,
}

/// Tags UI spawned for the in-game screen, so it can be torn down together when
/// the game returns to the menu.
#[derive(Component)]
pub struct InGameUi;

/// Which flavour of lobby the player entered from the menu.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LobbyMode {
    /// A local game against AI; no networking.
    Local,
    /// Hosting a network game.
    Host,
    /// Joining a hosted network game.
    Client,
}
