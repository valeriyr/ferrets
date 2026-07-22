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

    /// Adds `ids` to the current selection for `player`, keeping existing order
    /// and skipping any already present.
    pub fn add(&mut self, player: PlayerId, ids: &[SimulationId]) {
        let selection = &mut self.0[player as usize];
        for &id in ids {
            if !selection.contains(&id) {
                selection.push(id);
            }
        }
    }

    /// Flips each of `ids` in the current selection for `player`: a present id is
    /// removed, an absent id is appended.
    pub fn toggle(&mut self, player: PlayerId, ids: &[SimulationId]) {
        let selection = &mut self.0[player as usize];
        for &id in ids {
            if let Some(pos) = selection.iter().position(|&s| s == id) {
                selection.remove(pos);
            } else {
                selection.push(id);
            }
        }
    }

    /// Removes each of `ids` from the current selection for `player`.
    pub fn subtract(&mut self, player: PlayerId, ids: &[SimulationId]) {
        self.0[player as usize].retain(|s| !ids.contains(s));
    }

    /// Removes `id` from every player's selection.
    ///
    /// Selections are independent per-player views and not exclusive: several
    /// players can have the same entity selected at once (an enemy unit being
    /// inspected, a neutral mine), so all of them are swept.
    pub fn remove(&mut self, id: SimulationId) {
        for selection in &mut self.0 {
            selection.retain(|&selected| selected != id);
        }
    }
}
