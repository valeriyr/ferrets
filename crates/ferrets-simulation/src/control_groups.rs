//! Control groups — saved selections each player can recall.

use bevy_ecs::prelude::*;

use crate::{session::player_id::PlayerId, simulation_id::SimulationId};

/// The number of control groups each player has.
pub const CONTROL_GROUP_COUNT: usize = 10;

/// Saved selections each player can recall, indexed by [`PlayerId`].
///
/// A control group is player-local intent recorded as synced input: assigning
/// stores whatever is selected, recalling re-selects it. Membership is not
/// exclusive — an entity may sit in several groups at once.
#[derive(Resource)]
pub struct ControlGroups(Vec<[Vec<SimulationId>; CONTROL_GROUP_COUNT]>);

impl ControlGroups {
    /// Creates empty control groups for each player.
    pub fn new(player_count: usize) -> Self {
        Self(
            (0..player_count)
                .map(|_| std::array::from_fn(|_| Vec::new()))
                .collect(),
        )
    }

    /// Returns the ids saved in `group` for `player`.
    ///
    /// Panics if `group` is not in `0..CONTROL_GROUP_COUNT`.
    pub fn get(&self, player: PlayerId, group: usize) -> &[SimulationId] {
        assert_group(group);
        &self.0[player as usize][group]
    }

    /// Replaces `group` for `player` with `ids`.
    ///
    /// Panics if `group` is not in `0..CONTROL_GROUP_COUNT`.
    pub fn assign(&mut self, player: PlayerId, group: usize, ids: Vec<SimulationId>) {
        assert_group(group);
        self.0[player as usize][group] = ids;
    }

    /// Adds `ids` to `group` for `player`, skipping any already present.
    ///
    /// Panics if `group` is not in `0..CONTROL_GROUP_COUNT`.
    pub fn append(&mut self, player: PlayerId, group: usize, ids: &[SimulationId]) {
        assert_group(group);
        let group = &mut self.0[player as usize][group];
        for &id in ids {
            if !group.contains(&id) {
                group.push(id);
            }
        }
    }

    /// Removes `id` from every group of every player, so a destroyed entity
    /// leaves no stale membership behind.
    pub fn remove(&mut self, id: SimulationId) {
        for player in &mut self.0 {
            for group in player {
                group.retain(|&member| member != id);
            }
        }
    }
}

/// Panics if `group` is not a valid control-group index.
fn assert_group(group: usize) {
    assert!(
        group < CONTROL_GROUP_COUNT,
        "control group {group} out of range (0..{CONTROL_GROUP_COUNT})"
    );
}
