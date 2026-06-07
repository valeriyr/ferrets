//! Tick counter advancement — the final step of each simulation tick.

use crate::session::GameSession;

/// Advances the simulation tick counter by one.
pub fn tick(session: &mut GameSession) {
    session.advance_tick();
}
