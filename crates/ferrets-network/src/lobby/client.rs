//! The client side of the lobby: mirrors the host's broadcast state.

use ferrets_simulation::session::ai_hosting::AiHosting;
use ferrets_simulation::session::player_slot::PlayerId;

use crate::control::{ControlChannel, ControlEvent};
use crate::message::control::{ControlMessage, LobbyMessage, Occupant, SlotInfo, UdpEntry};
use crate::peer::PeerId;
use crate::topology::Topology;

/// The host's start signal, surfaced to the client once the game begins. Carries
/// the mesh UDP endpoint table; empty for a host-star game.
#[derive(Debug, Clone, Default)]
pub struct Started {
    pub udp_table: Option<Vec<UdpEntry>>,
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
    slots: Vec<SlotInfo>,
    topology: Topology,
    ai_hosting: AiHosting,
    /// The host's start signal, set once it arrives.
    started: Option<Started>,
}

impl LobbyClient {
    /// Wraps a control channel already connected to the host.
    pub fn new(control: ControlChannel) -> Self {
        Self {
            control,
            slots: Vec::new(),
            topology: Topology::HostStar,
            ai_hosting: AiHosting::default(),
            started: None,
        }
    }

    /// Announces this client to the host: its build version (so the host can
    /// refuse a mismatch), the UDP port it offers for a direct mesh (`None` if it
    /// offers none), and an optional preferred race.
    pub fn join(
        &mut self,
        advertised_udp_port: Option<u16>,
        race: Option<&str>,
    ) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::Join {
                protocol_version: ferrets_simulation::PROTOCOL_VERSION.to_string(),
                advertised_udp_port,
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
                    LobbyMessage::LobbyState {
                        slots,
                        topology,
                        ai_hosting,
                    } => {
                        self.slots = slots;
                        self.topology = topology;
                        self.ai_hosting = ai_hosting;
                        changed = true;
                    }
                    LobbyMessage::Start { udp_table } => {
                        self.started = Some(Started { udp_table });
                        changed = true;
                    }
                    LobbyMessage::Rejected { peer, reason }
                        if peer == self.control.local_peer() =>
                    {
                        return PollOutcome::Rejected(reason);
                    }
                    _ => {}
                },
                // A client only ever connects to the host, so any disconnect is it.
                ControlEvent::Disconnected(_) => return PollOutcome::HostLost,
                _ => {}
            }
        }
        PollOutcome::Waiting { changed }
    }

    /// The latest mirrored slot list (empty until the first state arrives).
    pub fn slots(&self) -> &[SlotInfo] {
        &self.slots
    }

    /// The host's chosen topology.
    pub fn topology(&self) -> Topology {
        self.topology
    }

    /// The host's chosen AI hosting mode.
    pub fn ai_hosting(&self) -> AiHosting {
        self.ai_hosting
    }

    /// This client's own peer handle (assigned by the host on connect).
    pub fn control_peer(&self) -> PeerId {
        self.control.local_peer()
    }

    /// This client's own slot, found by the peer id it was assigned on connect.
    pub fn local_player(&self) -> Option<PlayerId> {
        let me = self.control.local_peer();
        self.slots
            .iter()
            .find(|info| info.occupant == Occupant::Human { peer: me })
            .map(|info| info.slot)
    }

    /// The host's start signal once it has arrived, or `None` until then.
    pub fn started(&self) -> Option<&Started> {
        self.started.as_ref()
    }

    /// Surrenders the control channel for a host-star gameplay channel.
    pub fn into_control(self) -> ControlChannel {
        self.control
    }
}
