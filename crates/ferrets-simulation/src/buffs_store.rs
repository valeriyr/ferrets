//! The buff store both buff sites share.
//!
//! It tracks only identity, stacks, and remaining time, and is generic over
//! the buff id kind so both sites share one stacking and expiry
//! implementation.

use crate::content::stack_rule::StackRule;

/// One active buff instance: its registered id (the stacking and removal
/// identity), its remaining ticks, and how many stacks are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveBuff<BuffId> {
    id: BuffId,
    remaining: Option<u32>,
    stacks: u32,
}

/// The active buffs of one carrier, keyed by the site's buff id kind — the
/// store inside the per-entity component and each player's slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuffsStore<BuffId> {
    active: Vec<ActiveBuff<BuffId>>,
}

impl<BuffId> Default for BuffsStore<BuffId> {
    fn default() -> Self {
        Self { active: Vec::new() }
    }
}

impl<BuffId: Copy + PartialEq> BuffsStore<BuffId> {
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
