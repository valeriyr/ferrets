//! The lobby/control protocol: messages a host and its clients exchange to agree
//! on the session before it starts (and to steer it afterwards).
//!
//! These ride the reliable control channel. Gameplay frames and checksums are a
//! separate concern ([`Message`](super::Message)).

use std::net::SocketAddr;

use ferrets_simulation::session::{
    drop_policy::DropPolicy, finish_policy::FinishPolicy, game_speed::GameSpeed,
    player_id::PlayerId, player_slot::TeamId,
};
use serde::{Deserialize, Serialize};

use crate::{peer::PeerId, session_mode::SessionMode};

/// Who occupies a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Occupant {
    /// Empty and waiting for a human to join; becomes [`Human`](Self::Human) on
    /// connect, and counts as closed if still empty when the game starts.
    Open,
    /// A networked human, identified by the peer that controls it.
    Human { peer: PeerId },
    /// An AI player. Whether its frames appear on the wire depends on the
    /// session mode's AI hosting: computed on every node and never relayed, or
    /// computed on the host node, which sends the AI's frames under the AI's
    /// own player id.
    Ai,
    /// A disabled slot: no player, no base, excluded from the running game.
    Closed,
}

/// One player slot's state in the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotInfo {
    /// The slot's dense [`PlayerId`].
    pub slot: PlayerId,
    /// Who fills the slot.
    pub occupant: Occupant,
    /// The chosen race id, if any.
    pub race: Option<String>,
    /// The team the slot belongs to. `None` means no team — the player is
    /// hostile to everyone.
    pub team: Option<TeamId>,
}

/// Everything the host decides and every client mirrors: the slot list plus
/// the session-wide choices. One value on the wire and in the mirrors, so
/// "what the lobby agreed" always travels — and is absent — as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyState {
    pub slots: Vec<SlotInfo>,
    /// The peers watching the game, kept apart from the player slots: they
    /// are the session's observers, not the game's participants. A watcher
    /// has no seat of its own to hold open or closed — one is admitted on
    /// request, while [`observer_limit`](Self::observer_limit) has room.
    pub observers: Vec<PeerId>,
    /// How many watchers the host admits. `0` turns watching off.
    pub observer_limit: u8,
    pub mode: SessionMode,
    pub drop_policy: DropPolicy,
    pub finish_policy: FinishPolicy,
}

/// Who proposed a session-level change — the two identities that exist at
/// the control plane: a player by its slot, or a watching node by the peer
/// its messages arrive from. Ordered with players first, so colliding
/// proposals resolved by "lowest wins" always favor a player over a watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Proposer {
    /// The node fielding this player.
    Player(PlayerId),
    /// A watching node.
    Observer(PeerId),
}

/// A peer's gameplay (UDP) endpoint, distributed to every peer before a mesh game
/// starts so each can address the others directly. An unspecified IP (`0.0.0.0`)
/// means the peer could only report its bind address; a receiver substitutes the
/// address it actually reached that peer at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpEntry {
    pub peer: PeerId,
    pub addr: SocketAddr,
}

/// A lobby-coordination message, exchanged while configuring the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyMessage {
    /// Client → host on connect: the client's build identity (the host refuses a
    /// mismatch — see [`ferrets_simulation::PROTOCOL_VERSION`]), the ports the
    /// client will accept direct traffic on for a mesh — UDP for gameplay, TCP
    /// for the decentralized control mesh (`None` if it offers none; each is
    /// used only when the host picks a mode that needs it), and an optional
    /// preferred race.
    Join {
        protocol_version: String,
        advertised_udp_port: Option<u16>,
        advertised_control_port: Option<u16>,
        race: Option<String>,
    },
    /// Host → all, re-sent on every change: the authoritative [`LobbyState`].
    /// A client finds its own slot by the peer id it was assigned on connect.
    /// Peers mirror it so their view is always current and their config is
    /// built before the game starts.
    State(LobbyState),
    /// Host → all: the named peer was refused (e.g. a build mismatch). Only the
    /// rejected client acts on it; the others ignore it.
    Rejected { peer: PeerId, reason: String },
    /// Client → host: a request to set a slot's race. The host validates it and
    /// re-broadcasts the [`State`](Self::State).
    RequestRace { slot: PlayerId, race: String },
    /// Client → host: a request to set a slot's team (`None` = no team). The
    /// host validates it and re-broadcasts the [`State`](Self::State).
    RequestTeam {
        slot: PlayerId,
        team: Option<TeamId>,
    },
    /// Client → host: a request to watch instead of play. The host admits it
    /// while the observer limit has room, vacating whatever player slot the
    /// sender held, and re-broadcasts the [`State`](Self::State); a request
    /// it cannot grant changes nothing.
    RequestObserve,
    /// Client → host: a watcher's request to play instead. The host grants it
    /// while a player slot is open and re-broadcasts the
    /// [`State`](Self::State); a request it cannot grant changes nothing.
    RequestPlay,
    /// Host → all: lock the lobby and begin. The state is already synced, so this
    /// carries only what the lobby broadcasts did not — the endpoint tables:
    /// UDP gameplay endpoints for a mesh game, and TCP control endpoints for a
    /// decentralized one.
    Start {
        udp_table: Option<Vec<UdpEntry>>,
        control_table: Option<Vec<UdpEntry>>,
    },
}

/// An in-game control message, exchanged while the session runs. These are
/// commands, not continuously re-spread state, so they only ever ride the
/// reliable control links — never the (possibly lossy) gameplay channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InGameMessage {
    /// Under host authority, host → all: `player` contributes no input from
    /// `tick` on — the authoritative drop of a stalled player. Applying it is
    /// `GameSession::drop_player`; a receiver that has not reached `tick` yet
    /// applies it all the same (the requirement is per tick, not per moment).
    DropAt { player: PlayerId, tick: u32 },
    /// Under peer authority, one node's observation of a stalled tick: it is
    /// blocked at `tick` and the players in `missing` have not delivered
    /// their frames for it. The drop commits once every live player outside
    /// `missing` reports the same observation. `voter` is the originator.
    StallVote {
        voter: PlayerId,
        tick: u32,
        missing: Vec<PlayerId>,
    },
    /// Under host authority, any node → host: a request to pause (`true`) or
    /// resume (`false`). The host turns it into an authoritative
    /// [`PauseAt`](Self::PauseAt).
    PauseRequest { paused: bool },
    /// Pause (`true`) or resume (`false`) the session, effective at `tick` on
    /// every node so the change is deterministic. Under host authority only
    /// the host emits it; under peer authority any node may propose, and
    /// proposals colliding on the same tick resolve by lowest
    /// `(proposer, paused)` everywhere.
    PauseAt {
        proposer: Proposer,
        tick: u32,
        paused: bool,
    },
    /// Under host authority, any node → host: a request to run at `speed`. The
    /// host turns it into an authoritative [`SpeedAt`](Self::SpeedAt).
    ///
    /// The engine accepts any positive factor and judges none of them: which
    /// speeds a game offers, and which of them it is willing to impose on other
    /// players, is the game's own rule, stated by the frontend that draws the
    /// ladder. So a client that ignores its own rule can request any speed —
    /// forged input like any other, and milder than the alternatives a peer in
    /// relayed lockstep already has.
    SpeedRequest { speed: GameSpeed },
    /// Run at `speed` from `tick` on, on every node — a wall-clock change only,
    /// tick-aligned so every node changes pace together. Under host authority
    /// only the host emits it; under peer authority any node may propose, and
    /// proposals colliding on the same tick resolve by lowest
    /// `(proposer, speed)` everywhere.
    SpeedAt {
        proposer: Proposer,
        tick: u32,
        speed: GameSpeed,
    },
    /// Any node → its control links, re-sent on an interval: the fastest speed
    /// the sender can actually sustain, derived from what a tick costs it. Soft
    /// state, not a decision — the latest value from each peer stands until
    /// replaced or aged out, and every node folds the minimum itself. The host
    /// node folds what it has heard into its own report, which is how the
    /// minimum crosses control links shaped as a star. Slowing down needs no
    /// agreement, so this never goes through the authority.
    CapacityReport { capacity: GameSpeed },
}

/// A message on the control channel, before or during the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMessage {
    /// Lobby coordination, before the game starts.
    Lobby(LobbyMessage),
    /// Game control, after the game starts.
    InGame(InGameMessage),
}
