//! Player ownership for simulation entities.

use bevy_ecs::prelude::*;

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

/// Returns `true` when both owners are present and different.
///
/// Neutral entities (no owner) are hostile to no one.
pub fn are_hostile(a: Option<&OwnerComponent>, b: Option<&OwnerComponent>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.player() != b.player(),
        _ => false,
    }
}
