//! Player ownership for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_content::affiliation::Affiliation;

use crate::session::{GameSession, player_id::PlayerId};

/// The player that owns this entity.
///
/// Entities without this component are neutral (resource nodes, critters, …).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerComponent {
    player: PlayerId,
}

impl OwnerComponent {
    /// Creates a new `OwnerComponent` with the given data.
    #[inline]
    pub fn new(player: PlayerId) -> Self {
        Self { player }
    }

    /// Returns the owning player.
    #[inline]
    pub fn player(&self) -> PlayerId {
        self.player
    }
}

/// Returns `true` when both owners are present and are players that are not
/// allied (see [`GameSession::are_allied`]).
///
/// Neutral entities (no owner) are hostile to no one, and allies are hostile to
/// each other no more than a player is to itself.
pub fn are_hostile(session: &GameSession, a: Option<PlayerId>, b: Option<PlayerId>) -> bool {
    admits(session, Affiliation::Enemy, a, b)
}

/// Whether `theirs` is whose a rule reading `affiliation` from `mine` names.
///
/// The one test behind every "whose" a capability declares — whom a transport
/// admits, whom a cast may aim at, whose field coverage a rule reads.
/// Unowned is nobody's: it is named by [`Affiliation::Anyone`] and by nothing
/// else, and an asker with no owner of its own names nobody but through the
/// same arm.
pub fn admits(
    session: &GameSession,
    affiliation: Affiliation,
    mine: Option<PlayerId>,
    theirs: Option<PlayerId>,
) -> bool {
    let (Some(mine), Some(theirs)) = (mine, theirs) else {
        return matches!(affiliation, Affiliation::Anyone);
    };
    match affiliation {
        Affiliation::Own => mine == theirs,
        Affiliation::Allied => session.are_allied(mine, theirs),
        Affiliation::Enemy => !session.are_allied(mine, theirs),
        Affiliation::Anyone => true,
    }
}
