//! Active buffs and debuffs on an entity.
//!
//! Active buffs are the source the stat pipeline folds into each entity's
//! effective stats (see [`StatsComponent::recompute`](super::entity_stats::StatsComponent::recompute)).

use bevy_ecs::prelude::*;

use crate::buffs_store::{BuffsStore, Term};
use ferrets_content::{entity_buffs::EntityBuffId, stack_rule::StackRule};

/// The active buffs on an entity.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct BuffsComponent(BuffsStore<EntityBuffId>);

impl BuffsComponent {
    /// Applies the buff `id` on the given term, resolving stacking against any
    /// active instance of the same id per `stack_rule`.
    pub fn apply(&mut self, id: EntityBuffId, stack_rule: StackRule, term: Term) {
        self.0.apply(id, stack_rule, term);
    }

    /// Removes every active instance of `id`. Returns `true` if any was removed.
    pub fn remove(&mut self, id: EntityBuffId) -> bool {
        self.0.remove(id)
    }

    /// Advances every term by one tick, dropping the timed buffs that ran out
    /// and returning the upkeeps whose payment falls due this tick, each with
    /// the number of stacks it is owed for.
    pub fn tick_down(&mut self) -> Vec<(EntityBuffId, u32)> {
        self.0.tick_down()
    }

    /// The active buffs as `(id, stacks)` pairs.
    pub fn active(&self) -> impl Iterator<Item = (EntityBuffId, u32)> + '_ {
        self.0.active()
    }

    /// `true` when no buffs are active.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
