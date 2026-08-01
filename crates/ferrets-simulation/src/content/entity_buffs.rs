//! The entity-buff vocabulary: handles and the timed modifier bundles content
//! declares for entities.
//!
//! An entity buff sits on the entity it is applied to and reaches only that
//! carrier — modifiers descend, never climb, so nothing here can touch the
//! owner's player stats.

use super::stack_rule::StackRule;
use super::stats::EntityModifier;

/// A handle to a registered entity buff, assigned in registration order.
///
/// Content declares entity buffs by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityBuffId(u16);

impl EntityBuffId {
    /// Creates an entity buff id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more entity buffs registered than EntityBuffId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The definition of a buff (or debuff) that sits on an entity: a bundle of
/// entity modifiers, a lifetime, and a stacking rule. Registered content,
/// referenced by [`EntityBuffId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityBuffDef {
    /// The modifiers this buff contributes to its carrier (negative magnitudes
    /// make a debuff).
    pub modifiers: Vec<EntityModifier>,
    /// Lifetime in ticks; `None` is permanent (removed only explicitly).
    pub duration: Option<u32>,
    /// How a repeat application of this kind combines.
    pub stack_rule: StackRule,
}
