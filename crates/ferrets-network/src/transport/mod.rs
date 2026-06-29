//! The transport abstraction: how serialized messages move between peers.
//!
//! Transports deal only in opaque bytes and peer handles — they never touch
//! `serde` or the wire format. That keeps each transport small and lets the
//! lockstep driver own all encoding.

pub mod error;
pub mod loopback;
pub(crate) mod socket_io;
pub mod tcp;
pub mod udp;

use crate::{peer::PeerId, transport::error::TransportError};

/// The result type related to transports.
pub type Result<T> = std::result::Result<T, TransportError>;

/// Aggregate connection status, for gating the start of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Still establishing connections to peers.
    Connecting,
    /// Connected and ready to exchange frames.
    Connected,
    /// All peers are gone.
    Disconnected,
}

/// Something a transport observed since the last poll.
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// A peer joined.
    PeerConnected(PeerId),
    /// A peer left.
    PeerDisconnected(PeerId),
    /// An opaque message arrived from `from`.
    Message { from: PeerId, bytes: Vec<u8> },
}

/// Moves opaque messages between peers. Implemented per transport medium.
pub trait NetworkTransport: Send {
    /// This endpoint's own peer handle.
    fn local_peer(&self) -> PeerId;

    /// Sends `bytes` to every connected peer.
    fn broadcast(&mut self, bytes: &[u8]) -> Result<()>;

    /// Returns everything observed since the last call (non-blocking).
    fn poll(&mut self) -> Vec<TransportEvent>;

    /// The currently connected peers (excluding this endpoint).
    fn peers(&self) -> &[PeerId];

    /// Aggregate connection status.
    fn state(&self) -> ConnectionState;

    /// The source address observed for `peer` when it connected, if the transport
    /// tracks it (a host over a connection-oriented medium). `None` otherwise.
    fn observed_addr(&self, _peer: PeerId) -> Option<std::net::SocketAddr> {
        None
    }
}
