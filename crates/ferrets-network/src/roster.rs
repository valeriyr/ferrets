//! The authoritative mapping of peers to player slots.

use ferrets_simulation::session::player_id::PlayerId;

use crate::peer::PeerId;

/// The authoritative peer ↔ player-slot mapping: an ordered list where the peer
/// at index `i` owns [`PlayerId`] `i`.
///
/// It must be identical on every peer (the simulation only agrees if a given
/// peer's frames land in the same slot everywhere), so the connection point (a
/// host, even for peer-to-peer) assigns a slot to each peer as it joins and
/// distributes the finished roster. It is never inferred from a node's own
/// transport connectivity, which is partial and ordered differently on each node.
#[derive(Debug, Clone, Default)]
pub struct Roster {
    /// Peers indexed by player slot. `None` marks a slot with no network peer (an
    /// AI or a closed seat), which still occupies a slot index in the session.
    ///
    /// Only players: an observer's peer appears in no roster — it is a
    /// transport subscriber, reached because gameplay sends are broadcasts to
    /// every connected peer, and waited on by nothing.
    peers: Vec<Option<PeerId>>,
}

impl Roster {
    /// Builds a roster from an authoritative, ordered peer list: the peer at
    /// index `i` owns [`PlayerId`] `i`. Every slot is networked.
    pub fn new(peers: Vec<PeerId>) -> Self {
        Self {
            peers: peers.into_iter().map(Some).collect(),
        }
    }

    /// Builds a roster where some slots have no network peer (`None` = AI or
    /// closed). The slot at index `i` owns [`PlayerId`] `i`.
    pub fn from_slots(peers: Vec<Option<PeerId>>) -> Self {
        Self { peers }
    }

    /// The number of player slots in the roster.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether the roster is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Returns the player slot that owns `peer`, or `None` if `peer` is unmapped.
    pub fn player_of(&self, peer: PeerId) -> Option<PlayerId> {
        self.peers
            .iter()
            .position(|&p| p == Some(peer))
            .map(|i| i as PlayerId)
    }

    /// Returns the peer that owns `player`, or `None` if the slot has no peer.
    pub fn peer_of(&self, player: PlayerId) -> Option<PeerId> {
        self.peers.get(player as usize).copied().flatten()
    }

    /// Returns `true` if `player` is backed by a network peer (not AI or closed).
    pub fn is_networked(&self, player: PlayerId) -> bool {
        matches!(self.peers.get(player as usize), Some(Some(_)))
    }
}
