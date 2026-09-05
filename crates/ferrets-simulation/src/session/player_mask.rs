//! Player sets as bit masks.

use crate::session::player_id::PlayerId;

/// A set of players, one bit per player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerMask(u32);

impl PlayerMask {
    /// No player.
    pub const EMPTY: Self = Self(0);

    /// The mask holding `player` alone.
    pub fn of(player: PlayerId) -> Self {
        assert!(
            (player as u32) < u32::BITS,
            "player {player} does not fit a PlayerMask"
        );
        Self(1 << player)
    }

    /// Whether `player` is in the mask.
    #[inline]
    pub fn contains(self, player: PlayerId) -> bool {
        self & Self::of(player) != Self::EMPTY
    }

    /// Whether no player is in the mask.
    #[inline]
    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }

    /// The players in the mask, ascending.
    pub fn players(self) -> impl Iterator<Item = PlayerId> {
        (0..u32::BITS)
            .filter(move |bit| self.0 & (1 << bit) != 0)
            .map(|bit| bit as PlayerId)
    }
}

impl std::ops::BitAnd for PlayerMask {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitOr for PlayerMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::Not for PlayerMask {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl std::ops::BitOrAssign for PlayerMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAndAssign for PlayerMask {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
