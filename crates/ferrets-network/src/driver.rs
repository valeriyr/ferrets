//! The transport-agnostic lockstep driver.
//!
//! Bridges a [`NetworkTransport`] to the simulation's input pipeline without
//! depending on Bevy. It holds no frame storage of its own — the caller (the Bevy
//! bridge) owns the single source of truth, `InputFrames`, reads the broadcast
//! window from it, and hands the frames here to encode and send. The driver
//! decodes incoming traffic into plain [`PlayerFrame`]s and sync/connection
//! events, translating transport [`PeerId`](crate::peer::PeerId)s to
//! [`PlayerId`]s.

use ferrets_simulation::{input::PlayerFrame, session::player_slot::PlayerId};

use crate::{
    message::{self, Message, gameplay::GameplayMessage},
    peer::{HOST_PEER, PeerId},
    role::Role,
    roster::Roster,
    transport::{ConnectionState, NetworkTransport, TransportEvent},
};

/// A state checksum reported by a peer for a given tick (for desync detection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerChecksum {
    pub player: PlayerId,
    pub tick: u32,
    pub hash: u64,
}

/// A connection-level change observed this drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionEvent {
    Connected(PlayerId),
    Disconnected(PlayerId),
}

/// Everything decoded from the transport since the last drain.
#[derive(Debug, Default)]
pub struct Received {
    /// Newly-learned players' frames, to feed `InputFrames::push_frame`.
    pub frames: Vec<PlayerFrame>,
    /// Remote state checksums, to compare against the local hash.
    pub checksums: Vec<PeerChecksum>,
    /// Connection changes.
    pub events: Vec<ConnectionEvent>,
}

/// Drives lockstep over a transport: encodes and sends the frames the caller
/// selected, and decodes incoming traffic, translating transport [`PeerId`](crate::peer::PeerId)s to
/// [`PlayerId`]s. Holds no frame storage — `InputFrames` is the single source.
pub struct LockstepDriver {
    transport: Box<dyn NetworkTransport>,
    role: Role,
    roster: Roster,
    local_player: PlayerId,
}

impl LockstepDriver {
    /// Creates a driver over `transport` with an explicit `role` and the
    /// authoritative `roster`. The local peer must appear in the roster.
    pub fn new(transport: Box<dyn NetworkTransport>, role: Role, roster: Roster) -> Self {
        let local_player = roster
            .player_of(transport.local_peer())
            .expect("local peer is always in the roster");
        Self {
            transport,
            role,
            roster,
            local_player,
        }
    }

    /// The simulation slot this client controls.
    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }

    /// The player controlled by the session host's node ([`HOST_PEER`]), if
    /// that slot exists in the roster.
    pub fn host_player(&self) -> Option<PlayerId> {
        self.roster.player_of(HOST_PEER)
    }

    /// Whether this node is the session host's node.
    pub fn is_host_node(&self) -> bool {
        self.is_host_peer(self.transport.local_peer())
    }

    /// Whether `peer` is the session host's node.
    pub fn is_host_peer(&self, peer: PeerId) -> bool {
        peer == HOST_PEER
    }

    /// The player controlled by transport peer `peer`, if any.
    pub fn player_of(&self, peer: PeerId) -> Option<PlayerId> {
        self.roster.player_of(peer)
    }

    /// The transport peer that controls `player`, if the slot has one.
    pub fn peer_of(&self, player: PlayerId) -> Option<PeerId> {
        self.roster.peer_of(player)
    }

    /// The number of player slots in the session.
    pub fn player_count(&self) -> usize {
        self.roster.len()
    }

    /// `true` if `player` is backed by a remote/network peer (vs. AI/idle).
    pub fn is_networked(&self, player: PlayerId) -> bool {
        self.roster.is_networked(player)
    }

    /// `true` if this node forwards other players' frames (its [`Role`] relays).
    pub fn relays(&self) -> bool {
        self.role.relays()
    }

    /// Aggregate connection status.
    pub fn state(&self) -> ConnectionState {
        self.transport.state()
    }

    /// Broadcasts the given frames to all connected peers. The caller selects
    /// what belongs on the wire (the local player's frames, plus other players'
    /// on a relaying node); re-sending the window each tick gives an unreliable
    /// transport redundancy, and applying a frame twice is a no-op downstream.
    pub fn broadcast_frames(&mut self, frames: Vec<PlayerFrame>) -> crate::Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let bytes = message::encode(&Message::Gameplay(GameplayMessage::Frames(frames)))?;
        Ok(self.transport.broadcast(&bytes)?)
    }

    /// Broadcasts a local state checksum for `tick`. Checksums are never relayed.
    pub fn send_checksum(&mut self, tick: u32, hash: u64) -> crate::Result<()> {
        let bytes = message::encode(&Message::Gameplay(GameplayMessage::Sync { tick, hash }))?;
        Ok(self.transport.broadcast(&bytes)?)
    }

    /// Polls the transport and decodes everything received since the last call.
    ///
    /// Each returned frame carries its originator's `player` (preserved across
    /// relay hops — the immediate sender may be a relay, not the originator), to
    /// be fed to `InputFrames::push_frame` (idempotent, so duplicate/relayed
    /// copies are safe). Checksums and connection events use the transport
    /// sender's slot.
    pub fn drain_received(&mut self) -> Received {
        let mut received = Received::default();
        for event in self.transport.poll() {
            match event {
                TransportEvent::PeerConnected(peer) => {
                    if let Some(player) = self.roster.player_of(peer) {
                        received.events.push(ConnectionEvent::Connected(player));
                    }
                }
                TransportEvent::PeerDisconnected(peer) => {
                    if let Some(player) = self.roster.player_of(peer) {
                        received.events.push(ConnectionEvent::Disconnected(player));
                    }
                }
                TransportEvent::Message { from, bytes } => {
                    self.handle_message(from, &bytes, &mut received);
                }
            }
        }
        received
    }

    /// Decodes one message and folds it into `received`.
    fn handle_message(&mut self, from: PeerId, bytes: &[u8], received: &mut Received) {
        match message::decode(bytes) {
            Ok(Message::Gameplay(GameplayMessage::Frames(frames))) => {
                for frame in frames {
                    // Trust the frame's own `player`: under relay the immediate
                    // sender is not the originator. Drop frames for unknown slots.
                    if (frame.player as usize) < self.roster.len() {
                        received.frames.push(frame);
                    }
                }
            }
            Ok(Message::Gameplay(GameplayMessage::Sync { tick, hash })) => {
                if let Some(player) = self.roster.player_of(from) {
                    received.checksums.push(PeerChecksum { player, tick, hash });
                }
            }
            // The gameplay channel never receives control traffic (the control
            // channel is demultiplexed off before reaching here).
            Ok(Message::Control(_)) => {}
            // Ignore undecodable messages; a stricter build could log.
            Err(_) => {}
        }
    }
}
