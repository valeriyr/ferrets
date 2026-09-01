//! A player slot which can be either free or occupied by a player.

use serde::{Deserialize, Serialize};

use crate::{
    map_data::{MapData, MapSlot},
    scenario::Scenario,
    session::{ai_vision::AiVision, player_type::PlayerType},
};

/// A player unique ID, used to identify players in the simulation and replays.
pub type PlayerId = u8;

/// A team a player belongs to. Players sharing a team are allies; a player with
/// no team (`None`) is hostile to everyone.
pub type TeamId = u8;

/// How an occupied slot participates in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Participation {
    /// A lobby-configured combatant, counted by win conditions.
    Player,
    /// A combatant injected by the game or the map outside the lobby: it owns
    /// entities and fights, but is excluded from victory accounting — it can
    /// neither win nor block another side's victory.
    Environment,
}

/// What occupies a slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Occupancy {
    /// An unoccupied lobby seat.
    Free,
    /// A lobby-configured combatant.
    Player {
        player_type: PlayerType,
        /// The race the player plays, by registered race name. `None` until chosen.
        race: Option<String>,
        /// The team the player belongs to. `None` means the player is on no team
        /// and is hostile to everyone else.
        team: Option<TeamId>,
    },
    /// A game/map-injected AI combatant outside the lobby. Always AI-driven,
    /// raceless (its entities come from map placements, not race-picked game
    /// rules), and on no team — hostile to everyone.
    Environment {
        /// How much of the map its brain observes, as the brain declares.
        vision: AiVision,
    },
}

/// A player slot which can be either free or occupied by a player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSlot {
    /// A unique slot identifier.
    id: PlayerId,
    /// What occupies this slot.
    occupancy: Occupancy,
}

impl PlayerSlot {
    /// Creates a free slot with the given ID.
    pub fn free(id: PlayerId) -> Self {
        Self {
            id,
            occupancy: Occupancy::Free,
        }
    }

    /// Creates an occupied lobby-player slot with the given ID, player type,
    /// race (`None` if not chosen yet — set later with
    /// [`set_race`](Self::set_race)), and team (`None` for no team).
    pub fn occupied(
        id: PlayerId,
        player_type: PlayerType,
        race: Option<&str>,
        team: Option<TeamId>,
    ) -> Self {
        Self {
            id,
            occupancy: Occupancy::Player {
                player_type,
                race: race.map(String::from),
                team,
            },
        }
    }

    /// Creates an occupied [`Participation::Environment`] slot with the given
    /// ID and the vision its brain declares: an AI combatant outside the
    /// lobby, raceless and on no team — hostile to every other player.
    pub fn environment(id: PlayerId, vision: AiVision) -> Self {
        Self {
            id,
            occupancy: Occupancy::Environment { vision },
        }
    }

    /// Returns the ID of this slot.
    pub fn id(&self) -> PlayerId {
        self.id
    }

    /// Returns how this slot participates in the game, or `None` if the slot
    /// is free — a vacant seat participates as nothing.
    pub fn participation(&self) -> Option<Participation> {
        match self.occupancy {
            Occupancy::Free => None,
            Occupancy::Player { .. } => Some(Participation::Player),
            Occupancy::Environment { .. } => Some(Participation::Environment),
        }
    }

    /// Returns the player type occupying this slot, or `None` if the slot is
    /// free. An environment slot is always AI-driven.
    pub fn player_type(&self) -> Option<PlayerType> {
        match self.occupancy {
            Occupancy::Free => None,
            Occupancy::Player { player_type, .. } => Some(player_type),
            Occupancy::Environment { vision } => Some(PlayerType::Ai { vision }),
        }
    }

    /// Returns the vision the slot's scripted occupant declares, or `None`
    /// when no script drives it — a human observes through its screen, a
    /// free seat through nothing.
    pub fn ai_vision(&self) -> Option<AiVision> {
        match self.occupancy {
            Occupancy::Free => None,
            Occupancy::Player { player_type, .. } => match player_type {
                PlayerType::Human => None,
                PlayerType::Ai { vision } => Some(vision),
            },
            Occupancy::Environment { vision } => Some(vision),
        }
    }

    /// Returns the race name the player plays, or `None` if not chosen.
    pub fn race(&self) -> Option<&str> {
        match &self.occupancy {
            Occupancy::Free | Occupancy::Environment { .. } => None,
            Occupancy::Player { race, .. } => race.as_deref(),
        }
    }

    /// Sets the race the player plays.
    ///
    /// Panics unless the slot is an occupied lobby player — only the lobby
    /// reconfigures a slot; everything else is fixed at construction.
    pub fn set_race(&mut self, race: impl Into<String>) {
        let Occupancy::Player {
            race: slot_race, ..
        } = &mut self.occupancy
        else {
            panic!("only an occupied lobby player slot can change race");
        };
        *slot_race = Some(race.into());
    }

    /// Returns the team the player belongs to, or `None` if on no team. An
    /// environment slot is always on no team.
    pub fn team(&self) -> Option<TeamId> {
        match self.occupancy {
            Occupancy::Player { team, .. } => team,
            Occupancy::Free | Occupancy::Environment { .. } => None,
        }
    }

    /// Sets the team the player belongs to (or no team, `None`).
    ///
    /// Panics unless the slot is an occupied lobby player — only the lobby
    /// reconfigures a slot; an environment is on no team by construction.
    pub fn set_team(&mut self, team: Option<TeamId>) {
        let Occupancy::Player {
            team: slot_team, ..
        } = &mut self.occupancy
        else {
            panic!("only an occupied lobby player slot can change team");
        };
        *slot_team = team;
    }
}

/// The vacant session slots for a map's seats: a free slot per player seat,
/// an environment slot per environment seat, indexed by slot id.
///
/// `environment_vision` is what the game's environment brain declares — the
/// map places the seats, the game assigns the brain, so the caller carries
/// the declaration in.
pub fn vacant_slots(map: &MapData, environment_vision: AiVision) -> Vec<PlayerSlot> {
    map.slots()
        .iter()
        .enumerate()
        .map(|(id, seat)| match seat {
            MapSlot::Player { .. } => PlayerSlot::free(id as PlayerId),
            MapSlot::Environment => PlayerSlot::environment(id as PlayerId, environment_vision),
        })
        .collect()
}

/// The session slots a scenario runs with: the vacant slots of its map, with
/// the authored cast seated.
///
/// Panics if a cast entry names a seat the map does not declare as a player
/// seat, or names a seat twice — the cast and the map are authored together,
/// so a mismatch is a bug in the scenario.
pub fn scenario_slots(scenario: &Scenario, environment_vision: AiVision) -> Vec<PlayerSlot> {
    let mut slots = vacant_slots(&scenario.map, environment_vision);

    for player in &scenario.players {
        let seat = slots.get_mut(player.seat as usize).unwrap_or_else(|| {
            panic!(
                "scenario '{}' casts seat {}, which the map does not declare",
                scenario.name, player.seat
            )
        });
        assert_eq!(
            *seat,
            PlayerSlot::free(player.seat),
            "scenario '{}' casts seat {} twice, or casts an environment seat",
            scenario.name,
            player.seat
        );
        *seat = PlayerSlot::occupied(
            player.seat,
            player.player_type,
            player.race.as_deref(),
            player.team,
        );
    }
    slots
}
