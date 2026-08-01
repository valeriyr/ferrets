//! Active buffs and debuffs on an entity.
//!
//! Active buffs are the source the stat pipeline folds into each entity's
//! effective stats (see [`StatsComponent::recompute`](super::entity_stats::StatsComponent::recompute)).

use bevy_ecs::prelude::*;

use crate::buffs_store::BuffsStore;
use crate::content::entity_buffs::EntityBuffId;
use crate::content::stack_rule::StackRule;

/// The active buffs on an entity.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct BuffsComponent(BuffsStore<EntityBuffId>);

impl BuffsComponent {
    /// Applies the buff `id` with the given lifetime, resolving stacking against
    /// any active instance of the same id per `stack_rule`.
    pub fn apply(&mut self, id: EntityBuffId, stack_rule: StackRule, duration: Option<u32>) {
        self.0.apply(id, stack_rule, duration);
    }

    /// Removes every active instance of `id`. Returns `true` if any was removed.
    pub fn remove(&mut self, id: EntityBuffId) -> bool {
        self.0.remove(id)
    }

    /// Decrements each timed buff by one tick and drops any that reached zero.
    /// Returns `true` if anything expired.
    pub fn tick_down(&mut self) -> bool {
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
