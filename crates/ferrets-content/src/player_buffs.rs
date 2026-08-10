//! The player-buff vocabulary: handles and the timed modifier bundles content
//! declares for players.
//!
//! A player buff sits on the player it is applied to; its player modifiers
//! reach the player's own stats and its entity modifiers descend to every unit
//! the player owns.

use crate::{
    stack_rule::StackRule,
    stats::{EntityModifier, PlayerModifier},
};

/// A handle to a registered player buff, assigned in registration order.
///
/// Content declares player buffs by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerBuffId(u16);

impl PlayerBuffId {
    /// Creates a player buff id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more player buffs registered than PlayerBuffId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The definition of a buff (or debuff) that sits on a player. Registered
/// content, referenced by [`PlayerBuffId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerBuffDef {
    /// The modifiers this buff contributes to the player's own stats.
    pub player_modifiers: Vec<PlayerModifier>,
    /// The modifiers this buff lays over every unit the player owns.
    pub entity_modifiers: Vec<EntityModifier>,
    /// Lifetime in ticks; `None` is permanent (removed only explicitly).
    pub duration: Option<u32>,
    /// How a repeat application of this kind combines.
    pub stack_rule: StackRule,
}
