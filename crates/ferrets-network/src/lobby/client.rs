//! The client side of the lobby: mirrors the host's broadcast state.

use std::net::{TcpListener, UdpSocket};

use ferrets_simulation::session::{player_id::PlayerId, player_slot::TeamId};

use crate::{
    control::{ControlChannel, ControlEvent},
    message::control::{ControlMessage, LobbyMessage, LobbyState, Occupant, SlotInfo, UdpEntry},
    peer::PeerId,
    transport::error::TransportError,
};

/// The host's start signal, surfaced to the client once the game begins.
/// Carries the endpoint tables the mode needs: UDP gameplay endpoints for a
/// mesh game, TCP control endpoints for a decentralized one; both empty for a
/// host-star game.
#[derive(Debug, Clone, Default)]
pub struct Started {
    pub udp_table: Option<Vec<UdpEntry>>,
    pub control_table: Option<Vec<UdpEntry>>,
}

/// What a [`poll`](LobbyClient::poll) surfaced this tick.
#[derive(Debug)]
pub enum PollOutcome {
    /// Still in the lobby; `changed` is true if the mirrored state was updated.
    Waiting { changed: bool },
    /// The host refused this client, with the reason (e.g. a build mismatch).
    Rejected(String),
    /// The host's connection dropped, so the lobby cannot continue.
    HostLost,
}

/// The client side of the lobby: mirrors the host's broadcast state.
pub struct LobbyClient {
    control: ControlChannel,
    /// The host's authoritative state, `None` until the first broadcast
    /// arrives — a client honestly does not know it before that.
    state: Option<LobbyState>,
    /// The gameplay socket bound at [`join`](Self::join) so its real port
    /// could be advertised; consumed when a mesh game starts, dropped for a
    /// host-star game.
    udp: Option<UdpSocket>,
    /// The control listener bound at [`join`](Self::join) for the same
    /// reason; consumed when a decentralized game starts, dropped otherwise.
    control_listener: Option<TcpListener>,
    /// The host's start signal, set once it arrives.
    started: Option<Started>,
}

impl LobbyClient {
    /// Wraps a control channel already connected to the host.
    pub fn new(control: ControlChannel) -> Self {
        Self {
            control,
            state: None,
            udp: None,
            control_listener: None,
            started: None,
        }
    }

    /// Announces this client to the host: its build version (so the host can
    /// refuse a mismatch), the ports it offers for direct mesh traffic, and an
    /// optional preferred race.
    ///
    /// The gameplay socket and the control listener are bound here so the
    /// advertised ports are always the real ones; both are kept until the game
    /// starts. An explicit `udp_port` is used exactly as given (an occupied
    /// port is an error — a configured port must not be silently substituted);
    /// `None` binds an ephemeral port. The control listener is always
    /// ephemeral.
    pub fn join(&mut self, udp_port: Option<u16>, race: Option<&str>) -> crate::Result<()> {
        let udp =
            crate::transport::udp::bind_gameplay_socket(udp_port).map_err(TransportError::from)?;
        let advertised_udp_port = Some(udp.local_addr().map_err(TransportError::from)?.port());
        self.udp = Some(udp);
        let listener = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
            .map_err(TransportError::from)?;
        let advertised_control_port =
            Some(listener.local_addr().map_err(TransportError::from)?.port());
        self.control_listener = Some(listener);
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::Join {
                protocol_version: ferrets_simulation::PROTOCOL_VERSION.to_string(),
                advertised_udp_port,
                advertised_control_port,
                race: race.map(str::to_string),
            }))
    }

    /// Requests that this client's slot use `race`.
    pub fn request_race(&mut self, race: &str) -> crate::Result<()> {
        if let Some(slot) = self.local_player() {
            self.control
                .send(&ControlMessage::Lobby(LobbyMessage::RequestRace {
                    slot,
                    race: race.to_string(),
                }))?;
        }
        Ok(())
    }

    /// Requests that this client's slot join `team` (`None` = no team).
    pub fn request_team(&mut self, team: Option<TeamId>) -> crate::Result<()> {
        if let Some(slot) = self.local_player() {
            self.control
                .send(&ControlMessage::Lobby(LobbyMessage::RequestTeam {
                    slot,
                    team,
                }))?;
        }
        Ok(())
    }

    /// Requests to watch instead of play (granted while the host's observer
    /// limit has room).
    pub fn request_observe(&mut self) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::RequestObserve))
    }

    /// Requests to play instead of watch (granted while a player slot is
    /// open).
    pub fn request_play(&mut self) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::RequestPlay))
    }

    /// Drains control events, applying lobby state to the mirror and surfacing
    /// the host's refusal or disconnect to the caller.
    pub fn poll(&mut self) -> PollOutcome {
        let mut changed = false;
        for event in self.control.poll() {
            match event {
                ControlEvent::Message {
                    message: ControlMessage::Lobby(message),
                    ..
                } => match message {
                    LobbyMessage::State(state) => {
                        self.state = Some(state);
                        changed = true;
                    }
                    LobbyMessage::Start {
                        udp_table,
                        control_table,
                    } => {
                        self.started = Some(Started {
                            udp_table,
                            control_table,
                        });
                        changed = true;
                    }
                    LobbyMessage::Rejected { peer, reason }
                        if peer == self.control.local_peer() =>
                    {
                        return PollOutcome::Rejected(reason);
                    }
                    // Another peer's rejection, plus the requests only ever sent
                    // client → host.
                    LobbyMessage::Rejected { .. }
                    | LobbyMessage::Join { .. }
                    | LobbyMessage::RequestRace { .. }
                    | LobbyMessage::RequestTeam { .. }
                    | LobbyMessage::RequestObserve
                    | LobbyMessage::RequestPlay => {}
                },
                // In-game control reaches a client only once it has started.
                ControlEvent::Message {
                    message: ControlMessage::InGame(_),
                    ..
                } => {}
                // A client only ever connects to the host, so any disconnect is it.
                ControlEvent::Disconnected(_) => return PollOutcome::HostLost,
                ControlEvent::Connected(_) => {}
            }
        }
        PollOutcome::Waiting { changed }
    }

    /// The latest mirrored slot list (empty until the first state arrives).
    pub fn slots(&self) -> &[SlotInfo] {
        self.state.as_ref().map_or(&[], |state| &state.slots)
    }

    /// The host's authoritative state, `None` until the first broadcast
    /// arrives.
    pub fn state(&self) -> Option<&LobbyState> {
        self.state.as_ref()
    }

    /// This client's own peer handle (assigned by the host on connect).
    pub fn control_peer(&self) -> PeerId {
        self.control.local_peer()
    }

    /// This client's own slot, found by the peer id it was assigned on connect.
    pub fn local_player(&self) -> Option<PlayerId> {
        let me = self.control.local_peer();
        self.slots()
            .iter()
            .find(|info| info.occupant == Occupant::Human { peer: me })
            .map(|info| info.slot)
    }

    /// Returns `true` while this client's peer watches.
    pub fn local_observes(&self) -> bool {
        self.observers().contains(&self.control.local_peer())
    }

    /// The peers watching the game — empty until the host's first broadcast.
    pub fn observers(&self) -> &[PeerId] {
        self.state
            .as_ref()
            .map(|state| state.observers.as_slice())
            .unwrap_or(&[])
    }

    /// The host's start signal once it has arrived, or `None` until then.
    pub fn started(&self) -> Option<&Started> {
        self.started.as_ref()
    }

    /// Surrenders the control channel, the gameplay socket, and the control
    /// listener bound at [`join`](Self::join) (`None` when the client never
    /// joined).
    pub fn into_parts(self) -> (ControlChannel, Option<UdpSocket>, Option<TcpListener>) {
        (self.control, self.udp, self.control_listener)
    }
}
