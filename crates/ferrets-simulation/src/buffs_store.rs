//! The buff store both buff sites share.
//!
//! It tracks only identity, stacks, and the term each instance runs on, and is
//! generic over the buff id kind so both sites share one stacking and expiry
//! implementation.

use ferrets_content::stack_rule::StackRule;

/// The term one buff instance runs on, with what is left of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Term {
    /// Until something removes it.
    Forever,
    /// For a while yet.
    For {
        /// Ticks until it runs out.
        remaining: u32,
    },
    /// Until a payment is missed.
    Upkeep {
        /// Ticks between payments.
        period: u32,
        /// Ticks until the next payment falls due.
        due_in: u32,
    },
}

/// One active buff instance: its registered id (the stacking and removal
/// identity), the term it runs on, and how many stacks are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveBuff<BuffId> {
    id: BuffId,
    term: Term,
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
    /// Applies the buff `id` on the given term, resolving stacking against any
    /// active instance of the same id per `stack_rule`. A refreshed or stacked
    /// instance runs on the fresh term, except an upkeep, which keeps counting
    /// down to the payment it was already due.
    pub fn apply(&mut self, id: BuffId, stack_rule: StackRule, term: Term) {
        if let Some(existing) = self.active.iter_mut().find(|a| a.id == id) {
            match stack_rule {
                StackRule::Ignore => {}
                StackRule::Refresh => existing.term = refreshed(existing.term, term),
                StackRule::StackToCap(cap) => {
                    existing.stacks = (existing.stacks + 1).min(cap.max(1));
                    existing.term = refreshed(existing.term, term);
                }
            }
        } else {
            self.active.push(ActiveBuff {
                id,
                term,
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

    /// Advances every term by one tick: a timed buff that runs out is dropped,
    /// and an upkeep whose payment falls due this tick is returned, in
    /// application order, with its next payment set a period away.
    pub fn tick_down(&mut self) -> Vec<(BuffId, u32)> {
        let mut due = Vec::new();
        for active in &mut self.active {
            match &mut active.term {
                Term::Forever => {}
                Term::For { remaining } => *remaining = remaining.saturating_sub(1),
                Term::Upkeep { period, due_in } => {
                    *due_in = due_in.saturating_sub(1);
                    if *due_in == 0 {
                        due.push((active.id, active.stacks));
                        *due_in = *period;
                    }
                }
            }
        }
        self.active
            .retain(|active| !matches!(active.term, Term::For { remaining: 0 }));
        due
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

/// The term an active instance on `existing` runs on once the buff is applied
/// again on `term`: a timed or open-ended one takes the fresh term, an upkeep
/// keeps its own, the next payment falling due when it was going to.
fn refreshed(existing: Term, term: Term) -> Term {
    match existing {
        Term::Forever | Term::For { .. } => term,
        Term::Upkeep { .. } => existing,
    }
}
