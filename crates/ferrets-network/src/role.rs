//! The role a node plays in a networked session, and its relay policy.

/// The role this node plays in the session — which also decides whether it
/// relays other players' frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A peer in a (possibly partial) mesh: links to the other peers directly and
    /// forwards frames it holds for other players, so a peer with no direct link
    /// to player X still receives X's frames.
    Peer,
    /// The star hub: relays every client's frames to all the other clients.
    Host,
    /// A star leaf: linked only to the host; never relays.
    Client,
}

impl Role {
    /// Whether a node in this role forwards other players' frames.
    pub fn relays(self) -> bool {
        match self {
            Self::Peer | Self::Host => true,
            Self::Client => false,
        }
    }
}
