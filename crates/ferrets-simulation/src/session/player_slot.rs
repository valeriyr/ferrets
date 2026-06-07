//! A player slot which can be either free or occupied by a player.

use crate::session::player_type::PlayerType;

/// A player unique ID, used to identify players in the simulation and replays.
pub type PlayerId = u8;

/// A player slot which can be either free or occupied by a player.
#[derive(Debug, Clone, Copy)]
pub struct PlayerSlot {
    /// A unique slot identifier.
    id: PlayerId,
    /// How this slot is occupied.
    ///
    /// `None` means the slot exists but is not currently occupied by any player.
    player_type: Option<PlayerType>,
}

impl PlayerSlot {
    /// Creates a free slot with the given ID.
    pub fn free(id: PlayerId) -> Self {
        Self {
            id,
            player_type: None,
        }
    }

    /// Creates an occupied slot with the given ID and player type.
    pub fn occupied(id: PlayerId, player_type: PlayerType) -> Self {
        Self {
            id,
            player_type: Some(player_type),
        }
    }

    /// Returns the ID of this slot.
    pub fn id(&self) -> PlayerId {
        self.id
    }

    /// Returns the player type occupying this slot, or `None` if the slot is free.
    pub fn player_type(&self) -> Option<PlayerType> {
        self.player_type
    }
}
