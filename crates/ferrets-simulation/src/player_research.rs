//! The researches each player has completed.
//!
//! A completed research is a durable per-player fact: it satisfies requirement
//! entries naming it and blocks researching the same thing again. In-progress
//! research is not stored here — it lives on the researching entity's order,
//! so a researcher that dies can never leave stale state behind.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::{
    events::{EventRecord, SimulationEvent},
    session::player_slot::PlayerId,
    simulation_id::SimulationId,
};
use ferrets_content::research::ResearchId;

/// The completed researches of all players in the session, indexed by
/// [`PlayerId`].
#[derive(Resource, Debug, Default)]
pub struct PlayerResearch(Vec<BTreeSet<ResearchId>>);

impl PlayerResearch {
    /// Creates an empty completed set for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![BTreeSet::new(); player_count])
    }

    /// Marks the research `id` completed for `player`.
    pub fn mark_completed(&mut self, player: PlayerId, id: ResearchId) {
        self.0[player as usize].insert(id);
    }

    /// Returns `true` if `player` has completed the research `id`.
    pub fn is_completed(&self, player: PlayerId, id: ResearchId) -> bool {
        self.0[player as usize].contains(&id)
    }

    /// The player's completed researches, in ascending id order.
    pub fn completed(&self, player: PlayerId) -> impl Iterator<Item = ResearchId> + '_ {
        self.0[player as usize].iter().copied()
    }
}

/// Records `research` as finished for `player` and announces it.
///
/// The announcing counterpart to [`PlayerResearch::mark_completed`], which
/// records the topic and says nothing.
pub fn complete(
    world: &mut World,
    player: PlayerId,
    research: ResearchId,
    researcher: Option<SimulationId>,
) {
    world
        .resource_mut::<PlayerResearch>()
        .mark_completed(player, research);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ResearchCompleted {
            player,
            research,
            researcher,
        });
}
