//! Buffs and debuffs: timed bundles of [`Modifier`]s applied to an entity.
//!
//! A buff is the temporary/conditional counterpart to a base stat: it carries a
//! set of modifiers, an optional tick duration (`None` = permanent), and a
//! stacking rule that decides what happens when a buff of the same kind is
//! applied again. A debuff is simply a buff whose modifiers are negative.
//!
//! Active buffs are the source the stat pipeline folds into each entity's
//! effective stats (see [`StatsComponent::recompute`](super::stats::StatsComponent::recompute)).

use bevy_ecs::prelude::*;

use super::stats::Modifier;

/// A handle to a registered buff kind, assigned in registration order.
///
/// Content declares buff kinds by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuffId(u16);

impl BuffId {
    /// Creates a buff id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more buffs registered than BuffId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What happens when a buff is applied to an entity that already carries one of
/// the same [`BuffId`]. There is no engine default — content declares
/// the rule per buff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackRule {
    /// Keep the single instance and reset its remaining duration.
    Refresh,
    /// Add a stack (its modifiers apply once more), up to `cap`, and refresh the
    /// duration.
    StackToCap(u32),
    /// Keep the existing instance unchanged; drop the new application.
    Ignore,
}

/// The definition of a buff (or debuff): a bundle of modifiers, a lifetime, and a
/// stacking rule. Registered content, referenced by [`BuffId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuffDef {
    /// The modifiers this buff contributes (negative magnitudes make a debuff).
    pub modifiers: Vec<Modifier>,
    /// Lifetime in ticks; `None` is permanent (removed only explicitly).
    pub duration: Option<u32>,
    /// How a repeat application of this kind combines.
    pub stack_rule: StackRule,
}

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
