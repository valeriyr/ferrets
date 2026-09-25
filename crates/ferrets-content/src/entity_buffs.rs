//! The entity-buff vocabulary: handles and the bundles of effects with terms
//! content declares for entities.
//!
//! An entity buff sits on the entity it is applied to and reaches only that
//! carrier — its effects descend, never climb, so nothing here can touch the
//! owner's player stats.

use crate::{cost::Cost, entity_effect::EntityEffect, stack_rule::StackRule};

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

/// How long a buff lasts once applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lasting {
    /// Until something removes it.
    Forever,
    /// For this many ticks.
    For(u32),
    /// As long as its carrier keeps paying: the costs are drawn once every
    /// `period` ticks, and the tick they cannot be paid, it ends.
    Upkeep {
        /// What each payment draws.
        costs: Vec<Cost>,
        /// Ticks between payments.
        period: u32,
    },
}

/// What cuts a buff short before its lasting runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interruption {
    /// The bearer fires an attack: a swing or a shot at its damage point,
    /// from the body's weapon or a turret's, whether or not it then lands.
    Attack,
    /// The bearer lands a cast.
    Cast,
    /// Anyone lands damage on the bearer: a hit, splash, or a damaging cast.
    /// A drain or an upkeep of the bearer's own is not a hit.
    Hit,
}

/// The definition of a buff (or debuff) that sits on an entity: what it does
/// to its carrier, and on what terms. Registered content, referenced by
/// [`EntityBuffId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityBuffDef {
    /// What the buff does to its carrier while it holds (a modifier with a
    /// negative magnitude makes a debuff).
    pub effects: Vec<EntityEffect>,
    /// How long it holds.
    pub lasting: Lasting,
    /// How a repeat application of this kind combines.
    pub stack_rule: StackRule,
    /// What cuts it short before its lasting runs out.
    pub interrupted_by: Vec<Interruption>,
}
