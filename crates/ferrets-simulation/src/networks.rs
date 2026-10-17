//! Network scopes: whose gateways of a network link with one another, formed
//! from their owners.

use ferrets_content::{affiliation::Affiliation, network::NetworkId, registry::ContentRegistry};

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
    match (registry.network_def(network).boarding(), owner) {
        (Affiliation::Anyone, _) => Some(NetworkScope::Everyone),
        (Affiliation::Own, Some(player)) => Some(NetworkScope::Player(player)),
        (Affiliation::Allied, Some(player)) => {
            match session.slot(player).and_then(|slot| slot.team()) {
                Some(team) => Some(NetworkScope::Team(team)),
                None => Some(NetworkScope::Player(player)),
            }
        }
        (Affiliation::Own | Affiliation::Allied, None) => None,
        (Affiliation::Enemy, _) => {
            unreachable!("the registry refuses a network admitting only enemies")
        }
    }
}
