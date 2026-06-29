//! In-process transport: endpoints wired by channels.
//!
//! No sockets, no threads, fully synchronous — ideal for deterministic
//! multi-session tests of the lockstep loop (including partial-connectivity
//! relay) and as the default for a local game.

use std::collections::BTreeMap;

use crossbeam_channel::{Receiver, Sender, unbounded};

use super::{ConnectionState, NetworkTransport, TransportEvent};
use crate::{peer::PeerId, transport::error::TransportError};

/// A message in flight to an endpoint's inbox: `(sender, bytes)`.
type Envelope = (PeerId, Vec<u8>);

/// One endpoint of an in-process group of peers.
pub struct LoopbackTransport {
    local: PeerId,
    /// Directly-connected peers, ascending.
    peers: Vec<PeerId>,
    /// One sender per connected peer, into that peer's inbox.
    outboxes: BTreeMap<PeerId, Sender<Envelope>>,
    /// This endpoint's inbox.
    inbox: Receiver<Envelope>,
    /// Whether the one-time `PeerConnected` events have been emitted.
    announced: bool,
}

impl LoopbackTransport {
    /// Creates a connected pair with peer ids `0` and `1`.
    pub fn pair() -> (Self, Self) {
        let mut group = Self::mesh(2);
        let b = group.pop().expect("mesh(2) yields two endpoints");
        let a = group.pop().expect("mesh(2) yields two endpoints");
        (a, b)
    }

    /// Creates `n` fully-connected endpoints with peer ids `0..n`.
    pub fn mesh(n: usize) -> Vec<Self> {
        let links = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b)));
        Self::partial_mesh(n, links)
    }

    /// Creates `n` endpoints connected only along the given `links` (each an
    /// unordered `(a, b)` pair of indices). Lets a test model a peer that has no
    /// direct link to another and must rely on a relay.
    pub fn partial_mesh(n: usize, links: impl IntoIterator<Item = (usize, usize)>) -> Vec<Self> {
        let channels: Vec<(Sender<Envelope>, Receiver<Envelope>)> =
            (0..n).map(|_| unbounded()).collect();

        let mut endpoints: Vec<Self> = (0..n)
            .map(|i| Self {
                local: i as PeerId,
                peers: Vec::new(),
                outboxes: BTreeMap::new(),
                inbox: channels[i].1.clone(),
                announced: false,
            })
            .collect();

        for (a, b) in links {
            let (a_id, b_id) = (a as PeerId, b as PeerId);
            endpoints[a].outboxes.insert(b_id, channels[b].0.clone());
            endpoints[a].peers.push(b_id);
            endpoints[b].outboxes.insert(a_id, channels[a].0.clone());
            endpoints[b].peers.push(a_id);
        }
        for endpoint in &mut endpoints {
            endpoint.peers.sort_unstable();
        }
        endpoints
    }
}

impl NetworkTransport for LoopbackTransport {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    fn broadcast(&mut self, bytes: &[u8]) -> crate::transport::Result<()> {
        for outbox in self.outboxes.values() {
            outbox
                .send((self.local, bytes.to_vec()))
                .map_err(|_| TransportError::InternalError("loopback peer dropped".into()))?;
        }
        Ok(())
    }

    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut events = Vec::new();
        if !self.announced {
            self.announced = true;
            events.extend(self.peers.iter().map(|&p| TransportEvent::PeerConnected(p)));
        }
        while let Ok((from, bytes)) = self.inbox.try_recv() {
            events.push(TransportEvent::Message { from, bytes });
        }
        events
    }

    fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    fn state(&self) -> ConnectionState {
        if self.peers.is_empty() {
            ConnectionState::Disconnected
        } else {
            ConnectionState::Connected
        }
    }
}
