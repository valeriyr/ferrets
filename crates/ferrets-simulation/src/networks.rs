//! Network scopes: whose gateways of a network link with one another, formed
//! from their owners.

use ferrets_content::{
    network::NetworkId,
    registry::ContentRegistry,
    relation::{Relation, Relations},
};

use crate::session::{GameSession, player_id::PlayerId, player_slot::TeamId};

/// Whose gateways of one network link with one another: the scope its
/// `boarding` forms from their owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NetworkScope {
    /// One player's gateways.
    Player(PlayerId),
    /// The gateways of every player on one team.
    Team(TeamId),
    /// Every gateway of the network, whoever owns it.
    Everyone,
}

/// The scope a gateway of `network` owned by `owner` links in, or `None` for
/// an ownerless gateway of a network whose `boarding` scopes by owner.
pub(crate) fn scope_of(
    registry: &ContentRegistry,
    session: &GameSession,
    network: NetworkId,
    owner: Option<PlayerId>,
) -> Option<NetworkScope> {
    let boarding = registry.network_def(network).boarding();
    match (boarding, owner) {
        (boarding, _) if boarding == Relations::EVERY => Some(NetworkScope::Everyone),
        (boarding, Some(player)) if boarding == Relations::FRIENDLY => {
            Some(match session.slot(player).and_then(|slot| slot.team()) {
                Some(team) => NetworkScope::Team(team),
                None => NetworkScope::Player(player),
            })
        }
        (boarding, Some(player)) if boarding == Relation::Own.into() => {
            Some(NetworkScope::Player(player))
        }
        (boarding, None) if boarding == Relations::FRIENDLY || boarding == Relation::Own.into() => {
            None
        }
        _ => unreachable!(
            "the registry admits a network boarded by its owner's own, its owner's side, or every relation"
        ),
    }
}
