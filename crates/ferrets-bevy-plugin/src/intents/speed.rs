//! The speed request.

use bevy::prelude::*;
use ferrets_simulation::session::{GameSession, game_speed::GameSpeed};

/// The local player's pending speed request, set by the frontend on a speed
/// key — the game's one way to ask for a speed, networked or not.
#[derive(Resource, Default)]
pub struct SpeedIntent(pub Option<GameSpeed>);

/// Applies the local player's speed request straight to the session — the path
/// for a game with no network control plane to route it through (gated off
/// while [`NetworkActive`](crate::network::NetworkActive), where the control
/// plane consumes the same intent instead). The game only ever states intent;
/// which mechanism applies it — and the invariants that ride on the choice —
/// stay the engine's.
pub fn apply_local_speed(mut session: ResMut<GameSession>, mut intent: ResMut<SpeedIntent>) {
    if let Some(speed) = intent.0.take() {
        session.set_speed(speed);
    }
}

/// (Re)installs the request's per-game state, called from
/// [`install_game_resources`](crate::install_game_resources) so no entry path
/// can forget it. A request left over from the last game must not steer this
/// one.
pub(crate) fn install_per_game(world: &mut World) {
    world.insert_resource(SpeedIntent::default());
}

/// Clears the request when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources) — see
/// [`pause::remove_per_game`](crate::intents::pause).
pub(crate) fn remove_per_game(world: &mut World) {
    world.insert_resource(SpeedIntent::default());
}
