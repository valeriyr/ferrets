//! The buff vocabulary: handles, stacking rules, and the timed modifier bundles
//! content declares.

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
