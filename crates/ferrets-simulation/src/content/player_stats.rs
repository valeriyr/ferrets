//! The player-stat vocabulary: handles and the engine's built-in player stats.
//!
//! One stat group among its peers: the typed id is a gate keeping this group's
//! handles apart from the others', while the shared machinery in
//! [`super::stats`] folds them all the same way.

use ferrets_math::FixedU64;

use crate::content::stats::{self, BuiltinStat};

/// A handle to a registered player stat, assigned in registration order.
///
/// The built-in player stats occupy the low ids given by the associated
/// constants; content-declared player stats follow in registration order.
/// Identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerStatId(u16);

impl PlayerStatId {
    /// Ceiling on the player's provided supply, however much content stands on
    /// the map. A player without the stat is uncapped.
    pub const MAX_SUPPLY: PlayerStatId = PlayerStatId(0);

    /// Creates a player stat id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more player stats registered than PlayerStatId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The built-in player stats, registered first and in this order, so their
/// assigned ids equal the [`PlayerStatId`] constants above.
pub(crate) const PLAYER_BUILTIN_STATS: [BuiltinStat<PlayerStatId>; 1] = [
    // Zero is a meaningful ceiling — nothing may be trained — so no floor.
    stats::builtin(PlayerStatId::MAX_SUPPLY, "max_supply", FixedU64::ZERO),
];

// Floors and names are looked up by `PlayerStatId::index`, so every entry must
// sit at the slot its own id names.
const _: () = {
    let mut index = 0;
    while index < PLAYER_BUILTIN_STATS.len() {
        assert!(PLAYER_BUILTIN_STATS[index].id.index() == index);
        index += 1;
    }
};

/// The smallest effective value the player stat at registration `index` may
/// fold to. Content-declared player stats carry no engine meaning, so they have
/// no floor beyond the non-negative clamp.
pub(crate) fn floor_of(index: usize) -> FixedU64 {
    match PLAYER_BUILTIN_STATS.get(index) {
        Some(builtin) => builtin.floor,
        None => FixedU64::ZERO,
    }
}
