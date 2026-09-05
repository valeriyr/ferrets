//! The host side of the lobby: the authoritative slot list and state broadcast.

use std::{collections::HashMap, net::SocketAddr};

use ferrets_simulation::session::{
    drop_policy::DropPolicy, finish_policy::FinishPolicy, player_id::PlayerId, player_slot::TeamId,
};

use crate::{
    control::{ControlChannel, ControlEvent},
    message::control::{ControlMessage, LobbyMessage, LobbyState, Occupant, SlotInfo, UdpEntry},
    peer::PeerId,
    session_mode::SessionMode,
};

/// The host side of the lobby.
pub struct LobbyHost {
    control: ControlChannel,
    state: LobbyState,
    /// The UDP port each client advertised in its `Join` (for a mesh game).
    udp_ports: HashMap<PeerId, u16>,
    /// The TCP control port each client advertised (for a decentralized game).
    control_ports: HashMap<PeerId, u16>,
}

impl LobbyHost {
    /// Opens a lobby of `capacity` slots. Slot `0` is the host (a human); the rest
    /// start [`Open`](Occupant::Open) with `default_race`.
    pub fn new(
        control: ControlChannel,
        mode: SessionMode,
        drop_policy: DropPolicy,
        finish_policy: FinishPolicy,
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
                team: None,
            })
            .collect();
        Self {
            control,
            state: LobbyState {
                slots,
                observers: Vec::new(),
                // Watching starts off; a game that offers it raises the limit
                // (see `set_observer_limit`).
                observer_limit: 0,
                mode,
                drop_policy,
                finish_policy,
            },
            udp_ports: HashMap::new(),
            control_ports: HashMap::new(),
        }
    }

    /// The slot this host's own peer occupies, or `None` while the host sits
    /// in a watching seat — an observer holds no slot.
    pub fn local_player(&self) -> Option<PlayerId> {
        let peer = self.local_peer();
        self.state
            .slots
            .iter()
            .find(|info| info.occupant == Occupant::Human { peer })
            .map(|info| info.slot)
    }

    /// The host's own transport peer.
    pub fn local_peer(&self) -> PeerId {
        self.control.local_peer()
    }

    /// The peers watching the game.
    pub fn observers(&self) -> &[PeerId] {
        &self.state.observers
    }

    /// Returns `true` while this host's own peer watches.
    pub fn local_observes(&self) -> bool {
        self.state.observers.contains(&self.local_peer())
    }

    /// Sets how many watchers are admitted (`0` turns watching off). Gates
    /// admission only: watchers already in stay when the limit drops below
    /// them. Re-broadcasts.
    pub fn set_observer_limit(&mut self, limit: u8) -> crate::Result<()> {
        self.state.observer_limit = limit;
        self.broadcast_state()
    }

    /// The current slot list.
    pub fn slots(&self) -> &[SlotInfo] {
        &self.state.slots
    }

    /// The chosen session mode.
    pub fn mode(&self) -> SessionMode {
        self.state.mode
    }

    /// The authoritative lobby state this host broadcasts.
    pub fn state(&self) -> &LobbyState {
        &self.state
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
        if let Some(info) = self.state.slots.get_mut(slot as usize) {
            info.occupant = occupant;
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Moves `peer` from playing to watching, admitted while the observer
    /// limit has room — whatever player slot it held reopens. The host's own
    /// peer moves like any other. A request that cannot be granted changes
    /// nothing. Re-broadcasts on a move.
    pub fn observe(&mut self, peer: PeerId) -> crate::Result<()> {
        if self.admit_watcher(peer) {
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Moves the watching `peer` back to the first open player slot. A
    /// request that cannot be granted changes nothing. Re-broadcasts on a
    /// move.
    pub fn play(&mut self, peer: PeerId) -> crate::Result<()> {
        if self.seat_watcher(peer) {
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Changes the session mode before the game starts. Re-broadcasts so
    /// clients mirror it.
    pub fn set_mode(&mut self, mode: SessionMode) -> crate::Result<()> {
        self.state.mode = mode;
        self.broadcast_state()
    }

    /// Changes the drop policy before the game starts. Re-broadcasts so
    /// clients mirror it.
    pub fn set_drop_policy(&mut self, drop_policy: DropPolicy) -> crate::Result<()> {
        self.state.drop_policy = drop_policy;
        self.broadcast_state()
    }

    /// Changes the finish policy before the game starts. Re-broadcasts so
    /// clients mirror it.
    pub fn set_finish_policy(&mut self, finish_policy: FinishPolicy) -> crate::Result<()> {
        self.state.finish_policy = finish_policy;
        self.broadcast_state()
    }

    /// Sets a slot's race. Re-broadcasts.
    pub fn set_race(&mut self, slot: PlayerId, race: &str) -> crate::Result<()> {
        if let Some(info) = self.state.slots.get_mut(slot as usize) {
            info.race = Some(race.to_string());
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Sets a slot's team (`None` = no team). Re-broadcasts.
    pub fn set_team(&mut self, slot: PlayerId, team: Option<TeamId>) -> crate::Result<()> {
        if let Some(info) = self.state.slots.get_mut(slot as usize) {
            info.team = team;
            self.broadcast_state()?;
        }
        Ok(())
    }

    /// Locks the lobby and tells every client to start, carrying the endpoint
    /// tables the mode needs (`udp_table` for a mesh game, `control_table` for
    /// a decentralized one).
    pub fn start(
        &mut self,
        udp_table: Option<Vec<UdpEntry>>,
        control_table: Option<Vec<UdpEntry>>,
    ) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::Start {
                udp_table,
                control_table,
            }))
    }

    /// The UDP endpoint a client will receive gameplay on: its observed IP paired
    /// with the port it advertised. `None` until both are known.
    pub fn client_udp_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        let port = *self.udp_ports.get(&peer)?;
        let ip = self.control.observed_addr(peer)?.ip();
        Some(SocketAddr::new(ip, port))
    }

    /// The TCP endpoint a client accepts control-mesh links on: its observed
    /// IP paired with the port it advertised. `None` until both are known.
    pub fn client_control_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        let port = *self.control_ports.get(&peer)?;
        let ip = self.control.observed_addr(peer)?.ip();
        Some(SocketAddr::new(ip, port))
    }

    /// Surrenders the control channel (so a host-star gameplay channel can reuse
    /// the same socket).
    pub fn into_control(self) -> ControlChannel {
        self.control
    }

    /// Seats a newly-connected peer in the first open player slot — or, with
    /// every player slot taken, among the watchers while the observer limit
    /// has room: a latecomer to a full game may still watch it.
    fn seat(&mut self, peer: PeerId) -> bool {
        if let Some(info) = self
            .state
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Open)
        {
            info.occupant = Occupant::Human { peer };
            return true;
        }
        self.admit_watcher(peer)
    }

    /// Admits `peer` among the watchers while the limit has room, reopening
    /// whatever player slot it held. Returns whether it was admitted.
    fn admit_watcher(&mut self, peer: PeerId) -> bool {
        if self.state.observers.contains(&peer)
            || self.state.observers.len() >= self.state.observer_limit as usize
        {
            return false;
        }
        if let Some(info) = self
            .state
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Human { peer })
        {
            info.occupant = Occupant::Open;
        }
        self.state.observers.push(peer);
        true
    }

    /// Seats the watching `peer` in the first open player slot. Returns
    /// whether the move happened.
    fn seat_watcher(&mut self, peer: PeerId) -> bool {
        if !self.state.observers.contains(&peer) {
            return false;
        }
        let Some(info) = self
            .state
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Open)
        else {
            return false;
        };
        info.occupant = Occupant::Human { peer };
        self.state.observers.retain(|&watcher| watcher != peer);
        true
    }

    /// Reopens whatever place a departed peer held — a player slot, or one
    /// among the watchers.
    fn unseat(&mut self, peer: PeerId) -> bool {
        self.udp_ports.remove(&peer);
        self.control_ports.remove(&peer);
        if let Some(info) = self
            .state
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Human { peer })
        {
            info.occupant = Occupant::Open;
            return true;
        }
        if self.state.observers.contains(&peer) {
            self.state.observers.retain(|&watcher| watcher != peer);
            return true;
        }
        false
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
                advertised_control_port,
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
                if let Some(port) = advertised_control_port {
                    self.control_ports.insert(from, port);
                }
                if let (Some(race), Some(info)) = (race, self.slot_of_mut(from)) {
                    info.race = Some(race);
                }
                Ok(true)
            }
            LobbyMessage::RequestRace { slot, race } => {
                // A client may only set the race of the slot it occupies.
                match self.state.slots.get_mut(slot as usize) {
                    Some(info) if info.occupant == (Occupant::Human { peer: from }) => {
                        info.race = Some(race);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            LobbyMessage::RequestTeam { slot, team } => {
                // A client may only set the team of the slot it occupies.
                match self.state.slots.get_mut(slot as usize) {
                    Some(info) if info.occupant == (Occupant::Human { peer: from }) => {
                        info.team = team;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            LobbyMessage::RequestObserve => Ok(self.admit_watcher(from)),
            LobbyMessage::RequestPlay => Ok(self.seat_watcher(from)),
            // The host originates these; a client never sends them to the host.
            LobbyMessage::State { .. }
            | LobbyMessage::Start { .. }
            | LobbyMessage::Rejected { .. } => Ok(false),
        }
    }

    fn slot_of_mut(&mut self, peer: PeerId) -> Option<&mut SlotInfo> {
        self.state
            .slots
            .iter_mut()
            .find(|info| info.occupant == Occupant::Human { peer })
    }

    fn broadcast_state(&mut self) -> crate::Result<()> {
        self.control
            .send(&ControlMessage::Lobby(LobbyMessage::State(
                self.state.clone(),
            )))
    }
}
