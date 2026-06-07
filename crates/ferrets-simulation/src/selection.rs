//! Player units selection — tracks which entities each player has currently selected.

use bevy_ecs::prelude::*;

use crate::{session::player_slot::PlayerId, simulation_id::SimulationId};

/// Units selection for all players in the session, indexed by [`PlayerId`].
#[derive(Resource)]
pub struct Selection(Vec<Vec<SimulationId>>);

impl Selection {
    /// Creates a selection list with an empty selection for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![Vec::new(); player_count])
    }

    /// Returns the current selection for `player`.
    pub fn get(&self, player: PlayerId) -> &[SimulationId] {
        &self.0[player as usize]
    }

    /// Replaces the current selection for `player`.
    pub fn set(&mut self, player: PlayerId, ids: Vec<SimulationId>) {
        self.0[player as usize] = ids;
    }
}
