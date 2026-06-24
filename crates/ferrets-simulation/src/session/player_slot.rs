//! A player slot which can be either free or occupied by a player.

use crate::session::player_type::PlayerType;

/// A player unique ID, used to identify players in the simulation and replays.
pub type PlayerId = u8;

/// A player slot which can be either free or occupied by a player.
#[derive(Debug, Clone)]
pub struct PlayerSlot {
    /// A unique slot identifier.
    id: PlayerId,
    /// How this slot is occupied.
    ///
    /// `None` means the slot exists but is not currently occupied by any player.
    player_type: Option<PlayerType>,
    /// The race the player plays, by registered race name. `None` until chosen.
    race: Option<String>,
}

impl PlayerSlot {
    /// Creates a free slot with the given ID.
    pub fn free(id: PlayerId) -> Self {
        Self {
            id,
            player_type: None,
            race: None,
        }
    }

    /// Creates an occupied slot with the given ID, player type, and race
    /// (`None` if not chosen yet — set later with [`set_race`](Self::set_race)).
    pub fn occupied(id: PlayerId, player_type: PlayerType, race: Option<&str>) -> Self {
        Self {
            id,
            player_type: Some(player_type),
            race: race.map(String::from),
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

    /// Returns the race name the player plays, or `None` if not chosen.
    pub fn race(&self) -> Option<&str> {
        self.race.as_deref()
    }

    /// Sets the race the player plays.
    pub fn set_race(&mut self, race: impl Into<String>) {
        self.race = Some(race.into());
    }
}
