//! The reliable control channel: the lobby protocol and in-game control.
//!
//! Wraps a transport and (de)serializes [`ControlMessage`]s over it. When the
//! control and gameplay channels share one socket, gameplay traffic seen here is
//! ignored — the gameplay channel decodes that.

use std::net::SocketAddr;

use crate::message::control::ControlMessage;
use crate::message::{self, Message};
use crate::peer::PeerId;
use crate::transport::{ConnectionState, NetworkTransport, TransportEvent};

/// Something the control channel observed since the last poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// A peer connected.
    Connected(PeerId),
    /// A peer disconnected.
    Disconnected(PeerId),
    /// A control message arrived from `from`.
    Message {
        from: PeerId,
        message: ControlMessage,
    },
}

/// A reliable channel carrying control traffic between a host and its clients.
pub struct ControlChannel {
    transport: Box<dyn NetworkTransport>,
}

impl ControlChannel {
    /// Wraps `transport` as a control channel.
    pub fn new(transport: Box<dyn NetworkTransport>) -> Self {
        Self { transport }
    }

    /// This endpoint's own peer handle.
    pub fn local_peer(&self) -> PeerId {
        self.transport.local_peer()
    }

    /// Aggregate connection status.
    pub fn state(&self) -> ConnectionState {
        self.transport.state()
    }

    /// The source address observed for `peer` on connect, if tracked.
    pub fn observed_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        self.transport.observed_addr(peer)
    }

    /// The peers this endpoint holds a direct link to (those it can reach
    /// without a relay hop).
    pub fn peers(&self) -> &[PeerId] {
        self.transport.peers()
    }

    /// Sends a control message to every connected peer (a host reaches its
    /// clients; a client reaches its host).
    pub fn send(&mut self, message: &ControlMessage) -> crate::Result<()> {
        let bytes = message::encode(&Message::Control(message.clone()))?;
        Ok(self.transport.broadcast(&bytes)?)
    }

    /// Drains and decodes everything received since the last call. Non-control
    /// traffic on the same socket is ignored.
    pub fn poll(&mut self) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        for event in self.transport.poll() {
            match event {
                TransportEvent::PeerConnected(peer) => {
                    events.push(ControlEvent::Connected(peer));
                }
                TransportEvent::PeerDisconnected(peer) => {
                    events.push(ControlEvent::Disconnected(peer));
                }
                TransportEvent::Message { from, bytes } => {
                    if let Ok(Message::Control(message)) = message::decode(&bytes) {
                        events.push(ControlEvent::Message { from, message });
                    }
                }
            }
        }
        events
    }

    /// Surrenders the underlying transport, so a host-star gameplay channel can
    /// reuse the same socket.
    pub fn into_transport(self) -> Box<dyn NetworkTransport> {
        self.transport
    }
}
