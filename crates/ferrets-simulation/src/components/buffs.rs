//! Active buffs and debuffs on an entity.
//!
//! Active buffs are the source the stat pipeline folds into each entity's
//! effective stats (see [`StatsComponent::recompute`](super::stats::StatsComponent::recompute)).

use bevy_ecs::prelude::*;

use crate::content::buffs::{BuffId, StackRule};

/// One active buff instance on an entity: its registered id (the stacking and
/// removal identity), its remaining ticks, and how many stacks are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveBuff {
    id: BuffId,
    remaining: Option<u32>,
    stacks: u32,
}

/// The active buffs on an entity.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct BuffsComponent {
    active: Vec<ActiveBuff>,
}

impl BuffsComponent {
    /// Applies the buff `id` with the given lifetime, resolving stacking against
    /// any active instance of the same id per `stack_rule`.
    pub fn apply(&mut self, id: BuffId, stack_rule: StackRule, duration: Option<u32>) {
        if let Some(existing) = self.active.iter_mut().find(|a| a.id == id) {
            match stack_rule {
                StackRule::Ignore => {}
                StackRule::Refresh => existing.remaining = duration,
                StackRule::StackToCap(cap) => {
                    existing.stacks = (existing.stacks + 1).min(cap.max(1));
                    existing.remaining = duration;
                }
            }
        } else {
            self.active.push(ActiveBuff {
                id,
                remaining: duration,
                stacks: 1,
            });
        }
    }

    /// Removes every active instance of `id`. Returns `true` if any was removed.
    pub fn remove(&mut self, id: BuffId) -> bool {
        let before = self.active.len();
        self.active.retain(|a| a.id != id);
        self.active.len() != before
    }

    /// Decrements each timed buff by one tick and drops any that reached zero.
    /// Returns `true` if anything expired.
    pub fn tick_down(&mut self) -> bool {
        for active in &mut self.active {
            if let Some(remaining) = active.remaining.as_mut() {
                *remaining = remaining.saturating_sub(1);
            }
        }
        let before = self.active.len();
        self.active.retain(|active| active.remaining != Some(0));
        self.active.len() != before
    }

    /// The active buffs as `(id, stacks)` pairs.
    pub fn active(&self) -> impl Iterator<Item = (BuffId, u32)> + '_ {
        self.active.iter().map(|a| (a.id, a.stacks))
    }

    /// `true` when no buffs are active.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}
