//! The client side of the lobby: mirrors the host's broadcast state.

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

/// The client side of the lobby: mirrors the host's broadcast state.
pub struct LobbyClient {
    control: ControlChannel,
    slots: Vec<SlotInfo>,
    topology: Topology,
    /// The host's start signal, set once it arrives.
    started: Option<Started>,
    /// Set once the host's connection drops; the lobby can't continue.
    host_lost: bool,
}

impl LobbyClient {
    /// Wraps a control channel already connected to the host.
    pub fn new(control: ControlChannel) -> Self {
        Self {
            control,
            slots: Vec::new(),
            topology: Topology::HostStar,
            started: None,
            host_lost: false,
        }
    }

    /// Announces this client to the host: the UDP port it offers for a direct mesh
    /// (`None` if it offers none) and an optional preferred race.
    pub fn join(
        &mut self,
        advertised_udp_port: Option<u16>,
        race: Option<&str>,
    ) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::Join {
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

    /// Drains control events. Returns `true` if the mirrored state changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for event in self.control.poll() {
            match event {
                ControlEvent::Message {
                    message: ControlMessage::Lobby(message),
                    ..
                } => match message {
                    LobbyMessage::LobbyState { slots, topology } => {
                        self.slots = slots;
                        self.topology = topology;
                        changed = true;
                    }
                    LobbyMessage::Start { udp_table } => {
                        self.started = Some(Started { udp_table });
                        changed = true;
                    }
                    _ => {}
                },
                // A client only ever connects to the host, so any disconnect is it.
                ControlEvent::Disconnected(_) => {
                    self.host_lost = true;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// Whether the host's connection has dropped, so the lobby can't continue.
    pub fn host_lost(&self) -> bool {
        self.host_lost
    }

    /// The latest mirrored slot list (empty until the first state arrives).
    pub fn slots(&self) -> &[SlotInfo] {
        &self.slots
    }

    /// The host's chosen topology.
    pub fn topology(&self) -> Topology {
        self.topology
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
