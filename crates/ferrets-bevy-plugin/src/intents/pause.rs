//! The pause request.

use bevy::prelude::*;
use ferrets_simulation::session::GameSession;

/// The local player's pending pause/resume request (`Some(paused)`), set by the
/// frontend on a pause key — the game's one way to ask for a pause, networked
/// or not.
#[derive(Resource, Default)]
pub struct PauseIntent(pub Option<bool>);

/// Applies the local player's pause request straight to the session — the path
/// for a game with no network control plane to route it through (gated off
/// while [`NetworkActive`](crate::network::NetworkActive), where the control
/// plane consumes the same intent instead). The game only ever states intent;
/// which mechanism applies it — and the invariants that ride on the choice —
/// stay the engine's.
pub fn apply_local_pause(mut session: ResMut<GameSession>, mut intent: ResMut<PauseIntent>) {
    if let Some(paused) = intent.0.take() {
        session.set_paused(paused);
    }
}

/// (Re)installs the request's per-game state, called from
/// [`install_game_resources`](crate::install_game_resources) so no entry path
/// can forget it. A request left over from the last game must not steer this
/// one.
pub(crate) fn install_per_game(world: &mut World) {
    world.insert_resource(PauseIntent::default());
}

/// Clears the request when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources). A request the
/// finished game never consumed — one made in the final frames of a networked
/// game, whose control plane had already stopped running — would otherwise be
/// applied to the pending session between games and carry into the next one.
pub(crate) fn remove_per_game(world: &mut World) {
    world.insert_resource(PauseIntent::default());
}
