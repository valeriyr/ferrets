//! The host side of the lobby: the authoritative slot list and state broadcast.

use std::collections::HashMap;
use std::net::SocketAddr;

use ferrets_simulation::session::ai_hosting::AiHosting;
use ferrets_simulation::session::player_slot::PlayerId;

use crate::control::{ControlChannel, ControlEvent};
use crate::message::control::{ControlMessage, LobbyMessage, Occupant, SlotInfo, UdpEntry};
use crate::peer::PeerId;
use crate::topology::Topology;

/// The host side of the lobby.
pub struct LobbyHost {
    control: ControlChannel,
    topology: Topology,
    ai_hosting: AiHosting,
    slots: Vec<SlotInfo>,
    /// The UDP port each client advertised in its `Join` (for a mesh game).
    udp_ports: HashMap<PeerId, u16>,
}

impl LobbyHost {
    /// Opens a lobby of `capacity` slots. Slot `0` is the host (a human); the rest
    /// start [`Open`](Occupant::Open) with `default_race`.
    pub fn new(
        control: ControlChannel,
        topology: Topology,
        ai_hosting: AiHosting,
        capacity: usize,
        default_race: &str,
    ) -> Self {
        let host_peer = control.local_peer();
        let slots = (0..capacity)
            .map(|slot| SlotInfo {
                slot: slot as PlayerId,
                occupant: if slot == 0 {
                    Occupant::Human { peer: host_peer }
                } else {
                    Occupant::Open
                },
                race: Some(default_race.to_string()),
            })
            .collect();
        Self {
            control,
            topology,
            ai_hosting,
            slots,
            udp_ports: HashMap::new(),
        }
    }

    /// The host always controls slot `0`.
    pub fn local_player(&self) -> PlayerId {
        0
    }

    /// The current slot list.
    pub fn slots(&self) -> &[SlotInfo] {
        &self.slots
    }

    /// The chosen in-game topology.
    pub fn topology(&self) -> Topology {
        self.topology
    }

    /// The chosen AI hosting mode.
    pub fn ai_hosting(&self) -> AiHosting {
        self.ai_hosting
    }

    /// Drains control events, updating the lobby. Returns `true` if the state
    /// changed (and was re-broadcast), so the caller can refresh its view.
    pub fn poll(&mut self) -> crate::Result<bool> {
        let mut changed = false;
        for event in self.control.poll() {
            match event {
                ControlEvent::Connected(peer) => changed |= self.seat(peer),
                ControlEvent::Disconnected(peer) => changed |= self.unseat(peer),
                ControlEvent::Message { from, message } => {
                    changed |= self.apply(from, message)?;
                }
            }
        }
        if changed {
            self.broadcast_state()?;
        }
        Ok(changed)
    }

    /// Sets a slot the host controls to `occupant` (e.g. opening, closing, or
    /// making it an AI). Re-broadcasts.
    pub fn set_occupant(&mut self, slot: PlayerId, occupant: Occupant) -> crate::Result<()> {
        if let Some(info) = self.slots.get_mut(slot as usize) {
            info.occupant = occupant;
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Changes the in-game topology before the game starts. Re-broadcasts so
    /// clients mirror it.
    pub fn set_topology(&mut self, topology: Topology) -> crate::Result<()> {
        self.topology = topology;
        self.broadcast_state()
    }

    /// Changes the AI hosting mode before the game starts. Re-broadcasts so
    /// clients mirror it.
    pub fn set_ai_hosting(&mut self, ai_hosting: AiHosting) -> crate::Result<()> {
        self.ai_hosting = ai_hosting;
        self.broadcast_state()
    }

    /// Sets a slot's race. Re-broadcasts.
    pub fn set_race(&mut self, slot: PlayerId, race: &str) -> crate::Result<()> {
        if let Some(info) = self.slots.get_mut(slot as usize) {
            info.race = Some(race.to_string());
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Locks the lobby and tells every client to start, carrying `udp_table` (only
    /// meaningful for a mesh game).
    pub fn start(&mut self, udp_table: Option<Vec<UdpEntry>>) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::Start { udp_table }))
    }

    /// The UDP endpoint a client will receive gameplay on: its observed IP paired
    /// with the port it advertised. `None` until both are known.
    pub fn client_udp_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        let port = *self.udp_ports.get(&peer)?;
        let ip = self.control.observed_addr(peer)?.ip();
        Some(SocketAddr::new(ip, port))
    }

    /// Surrenders the control channel (so a host-star gameplay channel can reuse
    /// the same socket).
    pub fn into_control(self) -> ControlChannel {
        self.control
    }

    /// Seats a newly-connected peer in the first open slot.
    fn seat(&mut self, peer: PeerId) -> bool {
        if let Some(info) = self
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Open)
        {
            info.occupant = Occupant::Human { peer };
            true
        } else {
            false
        }
    }

    /// Reopens whatever slot a departed peer held.
    fn unseat(&mut self, peer: PeerId) -> bool {
        self.udp_ports.remove(&peer);
        if let Some(info) = self
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Human { peer })
        {
            info.occupant = Occupant::Open;
            true
        } else {
            false
        }
    }

    /// Applies a client's control message.
    fn apply(&mut self, from: PeerId, message: ControlMessage) -> crate::Result<bool> {
        let ControlMessage::Lobby(message) = message else {
            // In-game pause control is irrelevant before the game starts.
            return Ok(false);
        };
        match message {
            LobbyMessage::Join {
                protocol_version,
                advertised_udp_port,
                race,
            } => {
                if protocol_version != ferrets_simulation::PROTOCOL_VERSION {
                    // A different build would desync; refuse it and free its slot.
                    let reason = format!(
                        "build mismatch: host is {}, client is {protocol_version}",
                        ferrets_simulation::PROTOCOL_VERSION,
                    );
                    let reopened = self.unseat(from);
                    self.control
                        .send(&ControlMessage::Lobby(LobbyMessage::Rejected {
                            peer: from,
                            reason,
                        }))?;
                    return Ok(reopened);
                }
                if let Some(port) = advertised_udp_port {
                    self.udp_ports.insert(from, port);
                }
                if let (Some(race), Some(info)) = (race, self.slot_of_mut(from)) {
                    info.race = Some(race);
                }
                Ok(true)
            }
            LobbyMessage::RequestRace { slot, race } => {
                // A client may only set the race of the slot it occupies.
                match self.slots.get_mut(slot as usize) {
                    Some(info) if info.occupant == (Occupant::Human { peer: from }) => {
                        info.race = Some(race);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            // The host originates these; a client never sends them to the host.
            LobbyMessage::LobbyState { .. }
            | LobbyMessage::Start { .. }
            | LobbyMessage::Rejected { .. } => Ok(false),
        }
    }

    fn slot_of_mut(&mut self, peer: PeerId) -> Option<&mut SlotInfo> {
        self.slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Human { peer })
    }

    fn broadcast_state(&mut self) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::LobbyState {
                slots: self.slots.clone(),
                topology: self.topology,
                ai_hosting: self.ai_hosting,
            }))
    }
}
