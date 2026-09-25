//! Order queue component.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;

use crate::order::Order;

/// How a queued order responds to a cancel request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelPolicy {
    /// Advisory: the entry answers for itself, winding down at its next clean
    /// point or standing through the request.
    Soft,
    /// Mandatory: the entry ends on the tick the request is read, whether it
    /// had started or was still waiting its turn.
    Force,
}

impl CancelPolicy {
    /// Converts a boolean flush flag to an optional cancel policy.
    ///
    /// `true` → `Some(Soft)` (flush the queue, but let the front finish its step).
    /// `false` → `None` (append without canceling anything).
    pub fn from_bool(flush: bool) -> Option<Self> {
        if flush { Some(Self::Soft) } else { None }
    }

    /// The stronger of two policies: a mandatory cancel stands over an
    /// advisory one.
    pub fn stronger(self, other: Self) -> Self {
        match (self, other) {
            (Self::Force, Self::Force | Self::Soft) | (Self::Soft, Self::Force) => Self::Force,
            (Self::Soft, Self::Soft) => Self::Soft,
        }
    }
}

/// Processing state of a single order in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderState {
    /// Pushed but not yet started; prepared on the first tick it reaches the front.
    #[default]
    New,
    /// Currently being executed each tick.
    InProcessing,
    /// Paused while a sub-order executes at the front.
    Suspended,
    /// Execution complete; the entry will be popped by [`crate::game_loop::orders`].
    Finished,
}

/// One slot in an entity's order queue.
#[derive(Debug, Clone)]
pub struct OrderEntry {
    pub order: Order,
    pub state: OrderState,
    /// Set when this entry has been asked to stop; evaluated by [`crate::game_loop::orders::prepare_tick`].
    pub cancel: Option<CancelPolicy>,
}

impl OrderEntry {
    #[inline]
    pub fn new(order: Order) -> Self {
        Self {
            order,
            state: OrderState::New,
            cancel: None,
        }
    }
}

/// Ordered list of orders for an entity; the front entry executes each tick.
#[derive(Component, Debug, Default)]
pub struct OrderQueueComponent(pub VecDeque<OrderEntry>);

impl OrderQueueComponent {
    /// Appends a new order, optionally flushing the existing queue first.
    ///
    /// When `flush` is `Some(policy)`, all existing entries are marked for
    /// cancellation, an entry already marked keeping the stronger of its mark
    /// and `policy`. The actual cancellation is applied by
    /// [`crate::game_loop::orders::prepare_tick`] at the start of the next
    /// tick.
    pub fn push(&mut self, order: Order, flush: Option<CancelPolicy>) {
        if let Some(policy) = flush {
            self.mark_all(policy);
        }
        self.0.push_back(OrderEntry::new(order));
    }

    /// Prepends a new sub-order at the front.
    pub fn push_front(&mut self, order: Order) {
        self.0.push_front(OrderEntry::new(order));
    }

    /// Marks all entries for cancellation without adding a new order, an entry
    /// already marked keeping the stronger of its mark and `policy`.
    ///
    /// Use this for commands that stop without issuing a replacement.
    /// The actual cancellation is applied by [`crate::game_loop::orders::prepare_tick`]
    /// at the start of the next tick.
    pub fn cancel_all(&mut self, policy: CancelPolicy) {
        self.mark_all(policy);
    }

    /// Returns a reference to the front entry, if any.
    pub fn front(&self) -> Option<&OrderEntry> {
        self.0.front()
    }

    /// Returns a mutable reference to the front entry, if any.
    pub fn front_mut(&mut self) -> Option<&mut OrderEntry> {
        self.0.front_mut()
    }

    /// Pops the front entry, if any.
    pub fn pop_front(&mut self) {
        self.0.pop_front();
    }

    /// Marks every entry with `policy`, or with the stronger of `policy` and
    /// the mark it already carries.
    fn mark_all(&mut self, policy: CancelPolicy) {
        for entry in &mut self.0 {
            entry.cancel = Some(match entry.cancel {
                Some(marked) => marked.stronger(policy),
                None => policy,
            });
        }
    }
}
