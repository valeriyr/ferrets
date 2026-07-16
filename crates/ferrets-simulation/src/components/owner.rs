//! Player ownership for simulation entities.

use bevy_ecs::prelude::*;

use crate::session::GameSession;
use crate::session::player_slot::PlayerId;

/// The player that owns this entity.
///
/// Entities without this component are neutral (resource nodes, critters, …).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerComponent {
    player: PlayerId,
}

impl OwnerComponent {
    /// Creates a new `OwnerComponent` with the given data.
    #[inline]
    pub fn new(player: PlayerId) -> Self {
        Self { player }
    }

    /// Returns the owning player.
    #[inline]
    pub fn player(&self) -> PlayerId {
        self.player
    }
}

/// Returns `true` when both owners are present and belong to players that are
/// not allied (see [`GameSession::are_allied`]).
///
/// Neutral entities (no owner) are hostile to no one, and allies are hostile to
/// each other no more than a player is to itself.
pub fn are_hostile(
    session: &GameSession,
    a: Option<&OwnerComponent>,
    b: Option<&OwnerComponent>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => !session.are_allied(a.player(), b.player()),
        _ => false,
    }
}
