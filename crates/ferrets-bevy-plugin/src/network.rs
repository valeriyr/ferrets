//! Bevy wiring for lockstep networking.
//!
//! Bridges the [`NetSession`] to the simulation each `FixedUpdate`: [`net_receive`]
//! injects remote players' frames, [`flush_input`](crate::flush_input) records the
//! local frame, [`net_broadcast`] (re)broadcasts the frame window, [`net_checksum`]
//! exchanges per-tick state hashes to catch desyncs, and [`net_control`]
//! applies tick-aligned pauses and drops decided by the session's
//! [`Authority`]. Add it alongside
//! [`SimulationPlugin`](crate::SimulationPlugin) for every game: the systems run
//! only once a [`NetworkSession`] is installed, which a local game never does.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use bevy::prelude::*;
use ferrets_math::FixedU64;
use ferrets_network::{
    message::control::{ControlMessage, InGameMessage, Proposer},
    peer::PeerId,
    session::NetSession,
};
use ferrets_simulation::{
    checksum::{self, CHECKSUM_INTERVAL},
    input::{InputFrames, SYNC_LATENCY},
    session::{
        GameResult, GameSession, authority::Authority, drop_policy::DropPolicy,
        game_speed::GameSpeed, local_role::LocalRole, player_slot::PlayerId,
    },
};

use crate::{
    FixedUpdateSet,
    intents::{pause::PauseIntent, speed::SpeedIntent},
    session_is_active, session_is_advancing, session_is_not_paused,
    tick::{self, NominalTimestep, ThrottleConfig, TickPacing},
};

/// How far ahead of the deciding node's tick a session-level change takes
/// effect. Must exceed the inter-node tick spread (bounded by `SYNC_LATENCY`) so
/// no node has already passed that tick when the authoritative message is sent —
/// then every node reaches it and applies the change there. While the session is
/// paused no delay is possible: the tick is frozen, so a tick ahead of it never
/// arrives, and a change applies at the current one instead.
const CONTROL_DELAY: u32 = SYNC_LATENCY * 2;

/// Ticks between capacity reports, and the age at which a peer's last report is
/// forgotten. Reporting is paced in ticks rather than wall time so it needs no
/// clock; the game is not advancing anyway while nothing is being reported.
const CAPACITY_INTERVAL: u32 = 20;
/// The least a node will claim it can sustain. A [`GameSpeed`] cannot be zero, and
/// a node that cannot sustain even this is past accommodating — the drop rule's
/// business rather than the cadence's. The upper end needs no constant here: the
/// measurement is already capped at [`tick::MAX_FACTOR`].
const MIN_CAPACITY: FixedU64 = FixedU64::lit("0.001");
const CAPACITY_STALE_AFTER: u32 = CAPACITY_INTERVAL * 5;

/// Weight of a new sample when folding it into a player's running margin.
const MARGIN_SMOOTHING: FixedU64 = FixedU64::lit("0.2");
/// How long a player's last margin stands. A player keeping up refreshes it every
/// tick, so this only decides how quickly one that stopped sending — dropped,
/// eliminated, or gone — stops holding the cadence down.
const MARGIN_STALE_AFTER: u32 = CAPACITY_INTERVAL;

/// The reconnection grace window before a stall becomes a drop, in blocked
/// `FixedUpdate` steps (not wall-clock, so it is testable and only gates *when*
/// a node acts — generous enough to ride out a link blip). Whether the stall
/// becomes a decision at all is the session's [`DropPolicy`].
#[derive(Resource)]
pub struct DropConfig {
    pub timeout_steps: u32,
}

impl Default for DropConfig {
    fn default() -> Self {
        Self {
            // ~4 s at 20 Hz.
            timeout_steps: 80,
        }
    }
}

/// The stall currently blocking the tick, surfaced for the game's UI (a
/// wait-for-player dialog under [`DropPolicy::Manual`], a "connection lost"
/// toast otherwise). `None` while lockstep flows.
#[derive(Resource, Default)]
pub struct Stall(pub Option<StallInfo>);

/// One blocked tick's stall: which players are holding it up and for how long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallInfo {
    pub tick: u32,
    pub missing: Vec<PlayerId>,
    pub steps: u32,
}

/// The stalled players the local game has approved dropping, under
/// [`DropPolicy::Manual`]. The decision fires once every currently-missing
/// player is approved; cleared when it does.
#[derive(Resource, Default)]
pub struct DropIntent(pub Vec<PlayerId>);

/// The players whose control link to this node has gone down. A control link
/// is TCP, so this is definite knowledge, not a timeout guess: such a player
/// can neither receive decisions nor deliver its consensus vote here, so the
/// commit rules stop waiting for it. A genuinely dead peer loses its links to
/// every node at once, which is what keeps the exclusions converging.
#[derive(Resource, Default)]
pub struct ControlLinks {
    pub lost: BTreeSet<PlayerId>,
}

/// The stall observations known to this node under peer authority, by voter:
/// the blocked tick and the players the voter saw missing there. Each arrives
/// once over the reliable control mesh (a voter re-sends only when its
/// observation changes); cleared whenever the tick moves on.
#[derive(Resource, Default)]
pub struct StallVotes(pub BTreeMap<PlayerId, (u32, Vec<PlayerId>)>);

/// Tracks how many consecutive `FixedUpdate` steps the tick has been blocked at a
/// given tick, so [`detect_drops`] can fire once the grace window elapses.
#[derive(Resource, Default)]
pub struct BlockedStreak {
    tick: Option<u32>,
    steps: u32,
}

impl BlockedStreak {
    fn reset(&mut self) {
        self.tick = None;
        self.steps = 0;
    }
}

/// Holds the networked session (control + gameplay channels). A `NonSend`
/// resource because a transport need only be `Send`, not `Sync`. Absent until the
/// lobby starts a network game; the net systems are gated on [`NetworkActive`].
pub struct NetworkSession(pub NetSession);

/// A `Send` marker that a [`NetworkSession`] is installed. The net systems gate on
/// this rather than on the session directly, because run conditions may be
/// evaluated on worker threads where the `NonSend` session must not be touched.
#[derive(Resource)]
pub struct NetworkActive;

/// One scheduled change: who proposed it, what it is, and whether it has already
/// been handed out.
///
/// A claimed change is kept until its tick is *behind* the session, not
/// discarded when handed out: while the tick is frozen — which is exactly what a
/// pause does — a discarded entry would be re-learned from every flooded
/// duplicate and handed out again on every step.
#[derive(Clone, Copy)]
struct Scheduled<T> {
    proposer: Proposer,
    value: T,
    claimed: bool,
}

impl<T> Scheduled<T> {
    /// A change nobody has claimed yet — the only way one is born, so the flag
    /// is never a caller's to set.
    fn unclaimed(proposer: Proposer, value: T) -> Self {
        Self {
            proposer,
            value,
            claimed: false,
        }
    }
}

/// One node's proposal for a session-level change: who proposed it, the tick it
/// takes effect on, and what it is.
#[derive(Clone, Copy)]
struct Proposal<T> {
    /// Who proposed the change — a player, or a watching node by its peer:
    /// an observer host still steers the session it relays.
    proposer: Proposer,
    effective: u32,
    value: T,
}

/// Session-level changes scheduled to take effect at an agreed tick, identical
/// on every node so each change is deterministic. Applied (and discarded) by
/// [`net_control`] when a change's tick arrives; the control links are
/// reliable, so a proposal is sent exactly once.
///
/// One store per kind of change: two kinds proposed for the same tick must both
/// survive, and a single store keyed by tick alone would let one evict the other.
struct PendingChange<T>(BTreeMap<u32, Scheduled<T>>);

impl<T> Default for PendingChange<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<T: Copy + Ord> PendingChange<T> {
    /// Merges a proposal, returning whether it changed what is pending.
    /// Proposals for the same tick resolve identically on every node whatever
    /// their arrival order: the smallest `(player, value)` wins — and a winner
    /// arriving after the tick was already claimed is offered again, so every
    /// node converges on the same one.
    fn propose(&mut self, tick: u32, player: Proposer, value: T) -> bool {
        match self.0.entry(tick) {
            Entry::Vacant(entry) => {
                entry.insert(Scheduled::unclaimed(player, value));
                true
            }
            Entry::Occupied(mut entry) => {
                let held = *entry.get();
                let same = (player, value) == (held.proposer, held.value);
                let accept = if held.claimed {
                    // The tick's change was already handed out, and the entry is
                    // kept so a flooded duplicate is recognised — but only an
                    // *identical* message is that duplicate. A different one is a
                    // new decision landing on the same tick, which happens
                    // whenever the tick is frozen: a pause and the resume that
                    // lifts it are both stamped at the frozen tick, and refusing
                    // the second would leave the session unresumable by anyone
                    // the collision rule ranks after the pauser.
                    !same
                } else {
                    // Genuinely concurrent proposals for a tick not yet reached:
                    // the smallest wins, identically on every node.
                    (player, value) < (held.proposer, held.value)
                };
                if accept {
                    entry.insert(Scheduled::unclaimed(player, value));
                }
                accept
            }
        }
    }

    /// Hands out the newest change whose tick has arrived, **once** — a later
    /// call gets nothing for it, so a flooded duplicate arriving while the tick
    /// is frozen is recognised rather than handed out again. Each kind of change
    /// is a plain overwrite, so when a node crosses several effective ticks at
    /// once only the last decision stands.
    ///
    /// Whether the change is then applied is the caller's business; what this
    /// store guarantees is that it is offered exactly once.
    ///
    /// Advancing to `tick` also forgets the changes now behind the session.
    fn claim_due(&mut self, tick: u32) -> Option<T> {
        let due = self
            .0
            .range_mut(..=tick)
            .next_back()
            .filter(|(_, scheduled)| !scheduled.claimed)
            .map(|(_, scheduled)| {
                scheduled.claimed = true;
                scheduled.value
            });
        // Forgotten only after the due change is claimed: a tick the session
        // crossed in one jump (a seek) must still be handed out first.
        self.0.retain(|&effective, _| effective >= tick);
        due
    }
}

/// The scheduled pauses and resumes.
#[derive(Resource, Default)]
pub struct PendingPause(PendingChange<bool>);

/// The scheduled speed changes.
#[derive(Resource, Default)]
pub struct PendingSpeed(PendingChange<GameSpeed>);

/// How far ahead of the tick that needs them each player's frames are arriving,
/// smoothed, in ticks.
///
/// Steady transit latency lowers the lead without threatening the loop; the
/// warning sign is the margin decaying toward zero — frames arriving barely
/// before they are needed — which the blocked-streak only reports once the
/// stall has arrived (the boundary is
/// [`MARGIN_HEADROOM`](tick::MARGIN_HEADROOM)). Measured where frames land, so
/// it says how late they are *here* — under a host or a relayed mesh a frame
/// may have travelled through another peer, and this cannot tell the two apart.
#[derive(Resource, Default)]
pub struct FrameMargins(BTreeMap<PlayerId, (u32, FixedU64)>);

impl FrameMargins {
    /// Folds a frame's lead into the running margin for its player, as of `tick`.
    fn record(&mut self, player: PlayerId, tick: u32, lead: FixedU64) {
        match self.0.entry(player) {
            Entry::Vacant(entry) => {
                entry.insert((tick, lead));
            }
            Entry::Occupied(mut entry) => {
                let (_, running) = *entry.get();
                entry.insert((tick, tick::smooth(running, lead, MARGIN_SMOOTHING)));
            }
        }
    }

    /// The margin of the player, among those heard from recently, whose frames
    /// are arriving with the least room to spare — or `None` when nobody has been
    /// heard from. A player that stopped sending is left out: whatever its last
    /// margin was, it is no longer a statement about keeping up, and holding the
    /// game at that margin for the rest of the match would be a bug.
    pub fn tightest(&self, now: u32) -> Option<FixedU64> {
        self.0
            .values()
            .filter(|(at, _)| now.saturating_sub(*at) <= MARGIN_STALE_AFTER)
            .map(|(_, margin)| *margin)
            .min()
    }

    /// The margin last recorded for `player`, however long ago.
    pub fn of(&self, player: PlayerId) -> Option<FixedU64> {
        self.0.get(&player).map(|(_, margin)| *margin)
    }
}

/// The latest capacity each peer reported, with the tick it arrived on.
///
/// Soft state: a peer that recovers simply reports a higher value, so the fold
/// rises again on its own — which a one-shot decision could never do. A report
/// older than [`CAPACITY_STALE_AFTER`] is ignored, so a peer that stops talking
/// stops constraining the others.
#[derive(Resource, Default)]
pub struct PeerCapacities {
    /// What each peer last said it can sustain, with the tick it said it on.
    heard: BTreeMap<PlayerId, (u32, GameSpeed)>,
    /// The tick this node last published its own capacity at — the other half of
    /// the same conversation. Needed because `net_control` runs on every step:
    /// while the tick is frozen by a pause or a stalled peer, the reporting
    /// interval stays satisfied, and without this the node would republish on
    /// every one of those steps.
    sent_at: Option<u32>,
}

impl PeerCapacities {
    /// Records what `player` reported it can sustain, as of `tick`, replacing
    /// whatever it last said.
    pub fn record(&mut self, player: PlayerId, tick: u32, capacity: GameSpeed) {
        self.heard.insert(player, (tick, capacity));
    }

    /// Whether this node should publish its capacity at `tick`, claiming the
    /// slot so it publishes once per tick value rather than once per step spent
    /// there.
    pub fn claim_report_slot(&mut self, tick: u32) -> bool {
        if self.sent_at == Some(tick) {
            return false;
        }
        self.sent_at = Some(tick);
        true
    }

    /// The slowest speed any peer heard from recently can sustain — the ceiling
    /// the group as a whole can hold — or `None` when nobody has reported.
    pub fn tightest(&self, now: u32) -> Option<GameSpeed> {
        self.heard
            .values()
            .filter(|(at, _)| now.saturating_sub(*at) <= CAPACITY_STALE_AFTER)
            .map(|(_, capacity)| *capacity)
            .min()
    }
}

/// (Re)installs the control plane's per-game state, called from
/// [`install_game_resources`](crate::install_game_resources) so no entry path
/// can forget it. Stale votes or a peer still recorded as lost would otherwise
/// decide something in this game on the last one's evidence — and a lost host
/// id, in particular, aborts a session on sight. The margins and capacities are
/// stamped with the tick they arrived on, and ticks restart at zero, so a
/// survivor would never age out on its own.
pub(crate) fn install_per_game(world: &mut World) {
    world.insert_resource(ControlLinks::default());
    world.insert_resource(DesyncTracker::default());
    world.insert_resource(BlockedStreak::default());
    world.insert_resource(Stall::default());
    world.insert_resource(StallVotes::default());
    world.insert_resource(DropIntent::default());
    world.insert_resource(PendingPause::default());
    world.insert_resource(PendingSpeed::default());
    world.insert_resource(FrameMargins::default());
    world.insert_resource(PeerCapacities::default());
}

/// Installs the networked session: the `NonSend` session plus its `Send` marker.
/// Call at game start (the lobby does this) so the net systems begin running.
pub fn install_network_session(world: &mut World, session: NetSession) {
    world.insert_non_send_resource(NetworkSession(session));
    world.insert_resource(NetworkActive);
}

/// Removes the networked session when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources): a session left
/// installed would keep receiving into the menu and the next game.
pub(crate) fn remove_per_game(world: &mut World) {
    world.remove_non_send_resource::<NetworkSession>();
    world.remove_resource::<NetworkActive>();
}

/// Per-tick state hashes — local, and from each peer — for desync detection.
///
/// The peer side is keyed by `(player, tick)` so any number of peers can report.
/// Compared ticks are pruned each tick, so the maps stay bounded to the hashes
/// still awaiting a counterpart.
#[derive(Resource, Default)]
pub struct DesyncTracker {
    local: BTreeMap<u32, u64>,
    peer: BTreeMap<(PlayerId, u32), u64>,
}

impl DesyncTracker {
    /// Returns the earliest tick where some peer's hash disagrees with ours.
    fn first_mismatch(&self) -> Option<u32> {
        self.peer
            .iter()
            .filter(|((_, tick), peer_hash)| {
                self.local
                    .get(tick)
                    .is_some_and(|local| local != *peer_hash)
            })
            .map(|((_, tick), _)| *tick)
            .min()
    }

    /// Drops hashes for ticks already compared on both sides, keeping the maps
    /// bounded to the still-in-flight window. A tick present on both sides has
    /// been compared (a mismatch would have ended the game), so only ticks
    /// awaiting a counterpart need to be retained.
    fn prune_compared(&mut self) {
        let Some(&latest_local) = self.local.keys().next_back() else {
            return;
        };
        let Some(latest_peer) = self.peer.keys().map(|(_, tick)| *tick).max() else {
            return;
        };
        let compared = latest_local.min(latest_peer);
        self.local.retain(|&tick, _| tick > compared);
        self.peer.retain(|&(_, tick), _| tick > compared);
    }
}

/// Drives the ferrets simulation over a network transport.
///
/// Requires [`SimulationPlugin`](crate::SimulationPlugin). Registers the net
/// systems unconditionally; they run only once a [`NetworkSession`] is inserted
/// (by the lobby at game start), so this plugin is safe to add for every game,
/// networked or not.
#[derive(Default)]
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        // The per-game roster comes from the one function that owns it, so the
        // plugin cannot drift from what `install_game_resources` resets. Only
        // the game's own choice is created here and never overwritten later.
        app.init_resource::<DropConfig>();
        install_per_game(app.world_mut());
        // Order within the active tick: receive remote frames, then resolve
        // drops, record the local frame (flush_input), broadcast the window,
        // then checksum — all before command_executor consumes the input.
        // `net_receive` and `net_control` run even while paused, so frames and
        // control keep buffering and a resume can be received; everything that
        // advances the simulation is additionally gated on `session_is_not_paused`.
        app.add_systems(
            FixedUpdate,
            net_receive
                .in_set(FixedUpdateSet::Receive)
                .run_if(session_is_active.and(resource_exists::<NetworkActive>)),
        );
        app.add_systems(
            FixedUpdate,
            net_control
                .in_set(FixedUpdateSet::Receive)
                .after(net_receive)
                .run_if(session_is_active.and(resource_exists::<NetworkActive>)),
        );
        app.add_systems(
            FixedUpdate,
            detect_drops.in_set(FixedUpdateSet::Decide).run_if(
                session_is_active
                    .and(resource_exists::<NetworkActive>)
                    .and(session_is_not_paused),
            ),
        );
        app.add_systems(
            FixedUpdate,
            net_broadcast.in_set(FixedUpdateSet::Broadcast).run_if(
                session_is_active
                    .and(resource_exists::<NetworkActive>)
                    .and(session_is_not_paused),
            ),
        );
        app.add_systems(
            FixedUpdate,
            net_checksum
                .in_set(FixedUpdateSet::Broadcast)
                .after(net_broadcast)
                .run_if(session_is_advancing.and(resource_exists::<NetworkActive>)),
        );
    }
}

/// Applies the in-game control plane: tick-aligned pauses and player drops,
/// routed and decided according to the session's [`Authority`].
///
/// Under host authority the host is the single decider: it turns pause
/// requests into [`PauseAt`](InGameMessage::PauseAt) and stall decisions into
/// [`DropAt`](InGameMessage::DropAt) on the reliable control channel, and every
/// other node applies what arrives. Under peer authority there is no host to
/// relay through: a local pause intent becomes this node's tick-stamped
/// proposal on the control mesh (colliding proposals resolve by lowest
/// player id), and drops commit by consensus in [`detect_drops`].
///
/// Pending pause changes apply once their tick arrives, idempotently re-applied
/// and re-sent for a short tail so an unreliable transport still delivers them.
#[allow(clippy::too_many_arguments)]
pub fn net_control(
    mut net: NonSendMut<NetworkSession>,
    mut session: ResMut<GameSession>,
    mut pending: ResMut<PendingPause>,
    mut streak: ResMut<BlockedStreak>,
    mut votes: ResMut<StallVotes>,
    mut links: ResMut<ControlLinks>,
    mut pause_intent: ResMut<PauseIntent>,
    mut speeds: ResMut<PendingSpeed>,
    mut speed_intent: ResMut<SpeedIntent>,
    mut capacities: ResMut<PeerCapacities>,
    pacing: Res<TickPacing>,
    nominal: Res<NominalTimestep>,
    throttle_config: Res<ThrottleConfig>,
) {
    let host = net.0.is_host_node();
    let authority = session.authority();
    let tick = session.tick();
    let local = session.local_player();
    let stamp = local_proposer(&session, &net);
    // Whether this node is the one that turns a bare request into an
    // authoritative change: the host's own node under host authority, and
    // nobody under peer authority, where every node proposes for itself and no
    // request is ever sent.
    let decides = matches!(authority, Authority::Host { .. }) && host;

    // Drain the control links. Each message is judged against the peer that
    // actually sent it: a `PauseAt` schedules; a `DropAt` from the host node is
    // its authoritative drop — apply it, even for a tick this node has not
    // reached yet (but one for a tick already executed signals a divergence and
    // ends the game); a `StallVote` joins the known observations if the sender is
    // entitled to cast it; a `PauseRequest` (host authority only) queues for
    // the decision below. Under peer authority a vote or pause that taught this
    // node something new is forwarded once, so control commands cross a broken
    // link through the peers that still have one (duplicates change nothing and
    // are not re-forwarded, which ends the flood).
    let mut pause_requests: Vec<bool> = pause_intent.0.take().into_iter().collect();
    let mut speed_requests: Vec<GameSpeed> = speed_intent.0.take().into_iter().collect();
    let received = net.0.drain_control();
    // Record downed links before reading the messages, so a peer lost this same
    // drain is already known when its relayed vote is judged below.
    links.lost.extend(received.lost.iter().copied());
    for (from, message) in received.messages {
        if let ControlMessage::InGame(message) = message {
            match &message {
                // Only the node that decides acts on a bare request, and only the
                // host's own node decides. Without that gate every receiver
                // would queue the request *and* re-send it from `route_request`'s
                // non-host arm, so a forged request injected into a control mesh
                // would bounce between peers instead of being ignored.
                InGameMessage::PauseRequest { paused } => {
                    if decides {
                        pause_requests.push(*paused);
                    }
                }
                &InGameMessage::PauseAt {
                    proposer,
                    tick: effective,
                    paused,
                } => receive_change(
                    &mut net,
                    &mut pending.0,
                    &session,
                    from,
                    Proposal {
                        proposer,
                        effective,
                        value: paused,
                    },
                    message,
                ),
                // Same gate as a pause request: only the deciding node promotes
                // one. Which speeds are worth offering is the frontend's own
                // rule, stated by the ladder it draws.
                InGameMessage::SpeedRequest { speed } => {
                    if decides {
                        speed_requests.push(*speed);
                    }
                }
                InGameMessage::CapacityReport { capacity } => {
                    // Attributed to the peer that sent it, not to a field it
                    // could have filled in for somebody else. A node that does
                    // not take part in sharing ignores what it is told, just as
                    // it says nothing itself.
                    if let Some(player) = net.0.player_of(from)
                        && throttle_config.share_capacity
                    {
                        capacities.record(player, tick, *capacity);
                    }
                }
                &InGameMessage::SpeedAt {
                    proposer,
                    tick: effective,
                    speed,
                } => receive_change(
                    &mut net,
                    &mut speeds.0,
                    &session,
                    from,
                    Proposal {
                        proposer,
                        effective,
                        value: speed,
                    },
                    message,
                ),
                &InGameMessage::DropAt { player, tick: at } => {
                    // The authoritative drop, valid only under host authority
                    // and only from the host's own node. A client cannot drop a
                    // player by sending this, and under peer authority drops
                    // never travel this way — they commit by `StallVote`
                    // consensus in `detect_drops`. A player no slot seats is a
                    // corrupt message, not a drop. A player with a drop already
                    // decided — even one for a tick still ahead — is never
                    // re-dropped.
                    if !matches!(authority, Authority::Host { .. })
                        || !net.0.is_host_peer(from)
                        || session.slot(player).is_none()
                        || session.drop_tick(player).is_some()
                    {
                        continue;
                    }
                    if at < tick {
                        // The drop takes effect from a tick this node has
                        // already executed with the player's input. Drops are
                        // deterministic only because every survivor blocks at
                        // the same first unfillable tick before one is decided,
                        // so a drop older than the current tick means that
                        // convergence did not hold and our state past `at`
                        // disagrees with the authority's. Stop here rather than
                        // silently apply it and diverge further.
                        session.finish(GameResult::Desynchronization { tick: at });
                        continue;
                    }
                    session.drop_player(player, at);
                    streak.reset();
                }
                InGameMessage::StallVote {
                    voter,
                    tick,
                    missing,
                } => {
                    let (voter, tick) = (*voter, *tick);
                    if Some(voter) == local {
                        continue;
                    }
                    // Only the voter may originate its own vote. A relayed copy
                    // (the sender is not the voter) is the flood that carries a
                    // vote across a link this node lacks, so it is trusted only
                    // for a voter this node cannot hear directly — no direct
                    // control link, or one that has gone down. A relay about a
                    // voter on a live direct link is forged and dropped.
                    let relayed = net.0.player_of(from) != Some(voter);
                    let heard_directly =
                        net.0.has_control_link(voter) && !links.lost.contains(&voter);
                    if relayed && heard_directly {
                        continue;
                    }
                    let observation = (tick, missing.clone());
                    if votes.0.get(&voter) != Some(&observation) {
                        votes.0.insert(voter, observation);
                        match authority {
                            // Same relay split as a pause proposal.
                            Authority::Peers => forward(&mut net, message),
                            Authority::Host { .. } => {}
                        }
                    }
                }
            }
        }
    }

    // A downed control link is definite: the player behind it can no longer be
    // steered or consulted from here. Losing the host means losing the session
    // authority; losing every peer means this node cannot take part in any
    // decision — either way the session is unsteerable and ends locally.
    let unsteerable = match authority {
        Authority::Host { .. } => {
            !host
                && net
                    .0
                    .host_player()
                    .is_some_and(|player| links.lost.contains(&player))
        }
        Authority::Peers => {
            // A player out of the game steers nothing — its node is expected
            // to be gone, so a lost link to it proves no partition. Neither
            // does an observer: a watcher's link keeps no game alive.
            let others: Vec<PlayerId> = session
                .occupied_slots()
                .map(|slot| slot.id())
                .filter(|&player| {
                    Some(player) != local
                        && !session.is_player_out(player)
                        && net.0.is_networked(player)
                })
                .collect();
            !others.is_empty() && others.iter().all(|player| links.lost.contains(player))
        }
    };
    if unsteerable {
        session.finish(GameResult::Aborted);
        return;
    }

    for paused in pause_requests {
        // Pausing takes effect a margin ahead so every node freezes at the same
        // tick; resuming applies at the current (already frozen) tick.
        let effective = if session.is_paused() {
            tick
        } else {
            tick + CONTROL_DELAY
        };
        route_request(
            &mut net,
            &mut pending.0,
            &session,
            Proposal {
                proposer: stamp,
                effective,
                value: paused,
            },
            InGameMessage::PauseAt {
                proposer: stamp,
                tick: effective,
                paused,
            },
            InGameMessage::PauseRequest { paused },
        );
    }

    for speed in speed_requests {
        // A speed change is wall-clock only, so the margin buys alignment, not
        // correctness: every node changes pace at the same tick, so nobody is
        // briefly running a different cadence than the tick it is on. Unlike a
        // pause it is never stamped at a frozen tick — a change due at the tick
        // a concurrent resume unfreezes races that resume, and a node that moved
        // first would discard it as stale, leaving the speeds divergent for
        // good. A speed is inert while the tick is frozen anyway, so it loses
        // nothing by pending until the resumed loop reaches its tick.
        let effective = tick + CONTROL_DELAY;
        route_request(
            &mut net,
            &mut speeds.0,
            &session,
            Proposal {
                proposer: stamp,
                effective,
                value: speed,
            },
            InGameMessage::SpeedAt {
                proposer: stamp,
                tick: effective,
                speed,
            },
            InGameMessage::SpeedRequest { speed },
        );
    }

    // Tell the peers what this node can hold, on its interval. Soft state, so it
    // is re-sent rather than acknowledged, and a peer folds whatever it last
    // heard.
    // Once per interval, and once per tick: this system runs on every step, and a
    // tick frozen by a pause or a stalled peer would otherwise report on every
    // one of them.
    if let Some(nominal) = nominal.0
        && throttle_config.share_capacity
        && tick.is_multiple_of(CAPACITY_INTERVAL)
        && capacities.claim_report_slot(tick)
    {
        let sustainable = tick::sustainable_factor(pacing.exec_millis, tick::millis(nominal));
        let own = GameSpeed::new(sustainable.max(MIN_CAPACITY));
        // The host node reports the minimum of its own capacity and what it has
        // heard. Where the control links form a star, a client's report reaches
        // only the host, and this fold is what carries it on to the rest; a
        // report cannot be relayed verbatim, since it is attributed to its
        // sender. Everyone still recovers on its own: the fold reads each
        // player's *latest* report, and reports come from raw tick cost, so a
        // node that catches back up raises the minimum within an interval.
        let capacity = if host {
            capacities
                .tightest(tick)
                .map_or(own, |heard| own.min(heard))
        } else {
            own
        };
        if let Err(error) =
            net.0
                .send_control(&ControlMessage::InGame(InGameMessage::CapacityReport {
                    capacity,
                }))
        {
            eprintln!("failed to send capacity report: {error}");
        }
    }

    // Apply what each store has due, then discard it — the control links are
    // reliable, so nothing needs a resend tail.
    if let Some(paused) = pending.0.claim_due(tick) {
        session.set_paused(paused);
        // Clear any blocked-streak accrued across the pause boundary so a resume
        // does not immediately trip a drop.
        streak.reset();
    }
    if let Some(speed) = speeds.0.claim_due(tick) {
        session.set_speed(speed);
    }
}

/// Re-sends a just-learned control command on this node's own links (the
/// flooding step of peer-authority control).
fn forward(net: &mut NetworkSession, message: InGameMessage) {
    if let Err(error) = net.0.send_control(&ControlMessage::InGame(message)) {
        eprintln!("failed to forward control: {error}");
    }
}

/// The remote players' frame source: injects received frames into the input
/// queue and records peers' state checksums.
///
/// It does not synthesize frames for any slot, and a transport disconnect is not
/// treated as a game-ender (a lost link may still carry frames via relay). A
/// genuinely-gone player is handled by [`detect_drops`] once the tick has been
/// blocked on it past the grace window.
pub fn net_receive(
    mut net: NonSendMut<NetworkSession>,
    mut frames: ResMut<InputFrames>,
    mut tracker: ResMut<DesyncTracker>,
    mut margins: ResMut<FrameMargins>,
    session: Res<GameSession>,
) {
    let now = session.tick();
    let received = net.0.drain_received();
    for frame in received.frames {
        let (player, tick) = (frame.player, frame.tick);
        // A frame naming a player no slot seats is a corrupt message, not input:
        // recording it would index past the session's per-player stores and panic
        // the receiver. The same guard the authoritative drop gets.
        if session.slot(player).is_none() {
            continue;
        }
        // How much room the frame arrived with, counted only the first time it
        // shows up — recording it says whether that is now. Every node
        // rebroadcasts a whole window of frames each tick (see `net_broadcast`),
        // and a copy of something already held says nothing about whether its
        // sender is keeping up. The tick guard is for a frame that arrives after
        // its own tick executed, which only a dropped player's can.
        if frames.push_frame(frame) && tick >= now {
            margins.record(player, now, FixedU64::from_num(tick - now));
        }
    }
    for checksum in received.checksums {
        tracker
            .peer
            .insert((checksum.player, checksum.tick), checksum.hash);
    }
}

/// Resolves stalled players: once the tick has been blocked on them past the
/// grace window (or the game approved the drop under [`DropPolicy::Manual`]),
/// the session's [`Authority`] decides the drop — or the node aborts locally if
/// *every* other live player is missing (this node is partitioned — unless it
/// is the deciding host with live locally-sourced players left to steer) or
/// the deciding host is itself the one that stalled.
///
/// Deterministic in effect however it is decided: relay nodes rebroadcast the
/// whole frame window they hold every step (see [`net_broadcast`]), so a dying
/// player's final frames spread to every survivor well inside the grace window
/// — all survivors therefore advance to, and block at, the same first tick `B`
/// that nobody can fill, and compute the same missing set there. Dropping as of
/// `B` stops the tick from requiring the player's input, so ticks before `B`
/// executed its real frames everywhere and ticks from `B` on ignore it
/// everywhere — including any final frames that reached only some nodes. The
/// grace counter only gates *when* a node acts. (If a frame does arrive,
/// `command_executor` advances the tick and the streak resets before the
/// timeout — the automatic veto against dropping a merely-laggy player.)
///
/// Under host authority the drop is the host's [`DropAt`](InGameMessage::DropAt)
/// announcement; other nodes only apply it (in [`net_control`]). Under peer
/// authority each node casts its stall observation over the reliable control
/// mesh and the drop commits once every live player outside the missing set
/// reports the same one — unanimity that the relay convergence above makes
/// reachable.
// A Bevy system's parameters are its resource accesses, not an API surface.
#[allow(clippy::too_many_arguments)]
pub fn detect_drops(
    mut net: NonSendMut<NetworkSession>,
    frames: Res<InputFrames>,
    config: Res<DropConfig>,
    mut streak: ResMut<BlockedStreak>,
    mut stall: ResMut<Stall>,
    mut votes: ResMut<StallVotes>,
    mut intent: ResMut<DropIntent>,
    mut session: ResMut<GameSession>,
) {
    if !session.is_blocked() {
        streak.reset();
        stall.0 = None;
        votes.0.clear();
        return;
    }

    let tick = session.tick();
    if streak.tick == Some(tick) {
        streak.steps += 1;
    } else {
        streak.tick = Some(tick);
        streak.steps = 1;
        // Observations of an earlier blocked tick are stale, but a peer's vote
        // for *this* tick may already have arrived in `net_control` earlier
        // this step (it re-sends only on change, so dropping it would strand
        // the consensus). Keep those; discard the rest.
        votes.0.retain(|_, (voted_tick, _)| *voted_tick == tick);
    }

    let local = session.local_player();
    let is_host = net.0.is_host_node();
    // Only players whose frames must cross the network can stall this node: a
    // locally sourced slot (the local human, a replicated or host-side AI)
    // always has its frames, is never missing, and its node casts no vote of
    // its own — counting it would deadlock consensus and mask a partition.
    let live_others: Vec<PlayerId> = session
        .occupied_slots()
        .filter(|slot| !session.sources_locally(slot, is_host))
        .map(|slot| slot.id())
        .filter(|&player| awaits_frames(&session, player))
        .collect();
    let missing: Vec<PlayerId> = live_others
        .iter()
        .copied()
        .filter(|&player| !frames.has_frame(player, tick))
        .collect();

    if missing.is_empty() {
        stall.0 = None;
        return;
    }
    stall.0 = Some(StallInfo {
        tick,
        missing: missing.clone(),
        steps: streak.steps,
    });

    let grace_expired = streak.steps >= config.timeout_steps;
    if missing.len() == live_others.len() && !steers_local_players(&session, &net, is_host) {
        // Missing everyone reachable → this node is the one cut off; it cannot
        // determine a global tail (and no decision can reach it), so it ends
        // locally rather than dropping all — whatever the policy or authority.
        // The exception is a deciding host still fielding live locally-sourced
        // players: it has both the authority to drop and a game left to steer,
        // so it falls through to decide as usual.
        if grace_expired {
            session.finish(GameResult::Aborted);
            streak.reset();
        }
        return;
    }

    let decided = match session.drop_policy() {
        DropPolicy::Automatic => grace_expired,
        DropPolicy::Manual => missing.iter().all(|player| intent.0.contains(player)),
    };

    match session.authority() {
        Authority::Host { .. } if net.0.is_host_node() => {
            if !decided {
                return;
            }
            for &player in &missing {
                let message = InGameMessage::DropAt { player, tick };
                if let Err(error) = net.0.send_control(&ControlMessage::InGame(message)) {
                    eprintln!("failed to send drop control: {error}");
                }
                session.drop_player(player, tick);
            }
            intent.0.clear();
            streak.reset();
        }
        Authority::Host { .. } => {
            // Not this node's decision. But if the authority itself is the one
            // that stalled, no decision can ever arrive: the session cannot be
            // steered without its host, so it ends here.
            if grace_expired
                && net
                    .0
                    .host_player()
                    .is_some_and(|player| missing.contains(&player))
            {
                session.finish(GameResult::Aborted);
                streak.reset();
            }
        }
        Authority::Peers => {
            // Cast (or update) this node's observation, announcing it over the
            // reliable control mesh only when it changes.
            // An observer's node observes but holds no vote: consensus is the
            // players' unanimity, and no one counts a watcher.
            if decided && let Some(local) = local {
                let mine = (tick, missing.clone());
                if votes.0.get(&local) != Some(&mine) {
                    votes.0.insert(local, mine);
                    let message = InGameMessage::StallVote {
                        voter: local,
                        tick,
                        missing: missing.clone(),
                    };
                    if let Err(error) = net.0.send_control(&ControlMessage::InGame(message)) {
                        eprintln!("failed to send stall vote: {error}");
                    }
                }
            }
            // Unanimity: every live player outside the missing set — this node
            // included, when a player sits at it — reports exactly this stall.
            // A voter behind a single broken link still reaches here via the
            // flood; a voter whose control died entirely aborts itself, its
            // frames stop, and it joins the missing set — where its vote was
            // never required.
            let committed = live_others
                .iter()
                .copied()
                .filter(|player| !missing.contains(player))
                .chain(local)
                .all(|player| {
                    votes
                        .0
                        .get(&player)
                        .is_some_and(|(t, m)| *t == tick && *m == missing)
                });
            if committed {
                for &player in &missing {
                    session.drop_player(player, tick);
                }
                intent.0.clear();
                votes.0.clear();
                streak.reset();
            }
        }
    }
}

/// (Re)broadcasts the frame window read from `InputFrames` — the single source of
/// truth — around the current tick.
///
/// Selects only what belongs on the wire: network-backed players' frames (which
/// players those are depends on the session's AI hosting mode — a replicated
/// AI is computed on every node and never relayed), only frames some tick still
/// requires — a player's drop or elimination tick is known on every node, so a
/// frame at or past it is dead weight nothing will read, while the earlier
/// frames keep relaying until they leave the window (a lagging peer may still
/// need them) — and on a non-relay node only the players this node sources
/// (its own input, plus any AIs it computes for the others). Re-reading the
/// `[tick-SYNC_LATENCY, tick+SYNC_LATENCY]` window each tick is the redundancy
/// resend; idempotent `push_frame` makes duplicates harmless.
///
/// The relay is also what makes player drops land deterministically: a dying
/// player's final frames may reach nodes unevenly, and the relayed window
/// spreads them to every survivor, so all block at the same first unfillable
/// tick before the drop grace expires (see [`detect_drops`]).
pub fn net_broadcast(
    mut net: NonSendMut<NetworkSession>,
    frames: Res<InputFrames>,
    session: Res<GameSession>,
) {
    let tick = session.tick();
    let relays = net.0.relays();
    let is_host = net.0.is_host_node();

    let mut window = frames.frames_in_range(tick.saturating_sub(SYNC_LATENCY), tick + SYNC_LATENCY);
    window.retain(|frame| {
        let sourced = session
            .slot(frame.player)
            .is_some_and(|slot| session.sources_locally(slot, is_host));
        net.0.is_networked(frame.player)
            && session.is_player_required(frame.player, frame.tick)
            && (relays || sourced)
    });

    if let Err(error) = net.0.broadcast_frames(window) {
        // A broken transport stalls lockstep; surface it rather than diverging.
        eprintln!("failed to broadcast frames: {error}");
    }
}

/// Hashes the state entering this tick, broadcasts it at the checksum interval,
/// and ends the game if a peer's hash for any tick disagrees with ours.
pub fn net_checksum(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();

    if tick.is_multiple_of(CHECKSUM_INTERVAL) {
        let hash = checksum::state_checksum(world);
        world
            .resource_mut::<DesyncTracker>()
            .local
            .insert(tick, hash);
        if let Some(mut net) = world.get_non_send_resource_mut::<NetworkSession>()
            && let Err(error) = net.0.send_checksum(tick, hash)
        {
            eprintln!("failed to broadcast checksum: {error}");
        }
    }

    if let Some(bad_tick) = world.resource::<DesyncTracker>().first_mismatch() {
        eprintln!("desync detected at tick {bad_tick}; ending game");
        world
            .resource_mut::<GameSession>()
            .finish(GameResult::Desynchronization { tick: bad_tick });
    } else {
        world.resource_mut::<DesyncTracker>().prune_compared();
    }
}

/// A player whose frames the stall detector still waits for: no drop has been
/// decided (even one taking effect at a tick still ahead — that stall is
/// already handled), and the player is not eliminated as of the current tick —
/// an eliminated player's frames are required by nothing, so it neither blocks
/// the tick nor owes a consensus vote.
///
/// Not the negation of [`GameSession::is_player_out`]: that check is tick-aware
/// on both halves, so a player with a decided-but-pending drop is not yet out,
/// while this already treats their stall as handled.
fn awaits_frames(session: &GameSession, player: PlayerId) -> bool {
    session.drop_tick(player).is_none() && !session.is_player_eliminated(player)
}

/// Whether this node is the drop authority and still fields a live player
/// besides its own — a locally hosted AI or an environment slot — whose game
/// continues even with every remote player missing.
fn steers_local_players(session: &GameSession, net: &NetworkSession, is_host: bool) -> bool {
    matches!(session.authority(), Authority::Host { .. })
        && net.0.is_host_node()
        && session.occupied_slots().any(|slot| {
            Some(slot.id()) != session.local_player()
                && session.sources_locally(slot, is_host)
                && awaits_frames(session, slot.id())
        })
}

/// Who this node's own proposals are stamped as: its player, or — on a
/// watching node — its peer, the one identity a watcher has on the wire.
fn local_proposer(session: &GameSession, net: &NetworkSession) -> Proposer {
    match session.local_role() {
        LocalRole::Player(player) => Proposer::Player(player),
        LocalRole::Observer => Proposer::Observer(net.0.local_peer()),
    }
}

/// Merges one received tick-aligned proposal into its pending store: refuses a
/// sender the authority does not let decide, drops echoes and stale ticks, and
/// floods news — `message`, the proposal exactly as it arrived — onward on a
/// mesh.
///
/// Under host authority only the host node may announce a change — the guard
/// [`DropAt`](InGameMessage::DropAt) already has. A change any client could
/// inject would be applied here and forwarded nowhere, steering this node away
/// from every other.
///
/// An echo of this node's own proposal teaches nobody anything (the original
/// already went to every link), and a proposal for a tick already passed is a
/// stale copy of an applied-and-discarded change — re-learning either would
/// resurrect it in the apply loop. Legitimate traffic always targets the
/// present or future: the effective tick leads the proposer by more than the
/// lockstep skew.
fn receive_change<T: Copy + Ord>(
    net: &mut NetworkSession,
    pending: &mut PendingChange<T>,
    session: &GameSession,
    from: PeerId,
    proposal: Proposal<T>,
    message: InGameMessage,
) {
    let Proposal {
        proposer,
        effective,
        value,
    } = proposal;
    let authority = session.authority();
    if matches!(authority, Authority::Host { .. }) && !net.0.is_host_peer(from) {
        return;
    }
    if proposer == local_proposer(session, net) {
        return;
    }
    // A proposal for a tick already passed is a stale copy of a change that was
    // applied and is now dead; re-learning it would resurrect it. Converging a
    // genuinely late change instead would need each proposer's changes to carry
    // a sequence number, so a receiver could tell "newer than what I hold" from
    // "a duplicate of what I already applied" — which a tick alone cannot say.
    if effective < session.tick() {
        return;
    }
    if pending.propose(effective, proposer, value) {
        match authority {
            // A mesh has no relay of its own, so each node passes on what it
            // just learned; under host authority the host's own broadcast
            // already reached everybody.
            Authority::Peers => forward(net, message),
            Authority::Host { .. } => {}
        }
    }
}

/// Routes one locally-requested tick-aligned change. The deciding node — the
/// host under host authority, every node under peer authority — merges the
/// change into its own pending store and announces the authoritative `at`
/// message, so it applies at `effective` here and everywhere alike; colliding
/// proposals resolve identically on every node in `propose`. A client under
/// host authority sends the bare `request` instead and applies on the
/// authoritative answer.
fn route_request<T: Copy + Ord>(
    net: &mut NetworkSession,
    pending: &mut PendingChange<T>,
    session: &GameSession,
    proposal: Proposal<T>,
    at: InGameMessage,
    request: InGameMessage,
) {
    let deciding = match session.authority() {
        Authority::Host { .. } => net.0.is_host_node(),
        Authority::Peers => true,
    };
    let sent = if deciding {
        if !pending.propose(proposal.effective, proposal.proposer, proposal.value) {
            // Nothing changed here: either the tick already holds this very
            // change, or this proposal lost to one pending for the same tick.
            // Announcing it anyway would hand the peers a value this node is not
            // itself using — and a peer whose entry is already claimed accepts a
            // differing change, so it would act on it.
            return;
        }
        at
    } else {
        request
    };
    if let Err(error) = net.0.send_control(&ControlMessage::InGame(sent.clone())) {
        eprintln!("failed to send control {sent:?}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_ticks_both_sides_have_reported() {
        // Both reported through tick 16; 24 is local-only (peer's hash in flight).
        let mut tracker = tracker(&[(8, 1), (16, 2), (24, 3)], &[(1, 8, 1), (1, 16, 2)]);

        tracker.prune_compared();

        // Everything at or below the highest common tick (16) is gone; the
        // unmatched local tick stays until the peer's hash arrives.
        assert_eq!(tracker.local.keys().copied().collect::<Vec<_>>(), [24]);
        assert!(tracker.peer.is_empty());
    }

    #[test]
    fn prune_keeps_everything_until_peer_hash_arrives() {
        // No peer hashes yet: nothing has been compared, so nothing is dropped.
        let mut tracker = tracker(&[(8, 1), (16, 2)], &[]);

        tracker.prune_compared();

        assert_eq!(tracker.local.keys().copied().collect::<Vec<_>>(), [8, 16]);
    }

    #[test]
    fn first_mismatch_finds_earliest_disagreeing_tick() {
        // Peer 1 agrees at 8, disagrees at 16; peer 2 disagrees at 24.
        let tracker = tracker(
            &[(8, 1), (16, 2), (24, 3)],
            &[(1, 8, 1), (1, 16, 999), (2, 24, 999)],
        );

        assert_eq!(tracker.first_mismatch(), Some(16));
    }

    #[test]
    fn prune_never_discards_unmatched_mismatch() {
        // A mismatch at 8 is still detectable after pruning while 8 is unmatched.
        let mut tracker = tracker(&[(8, 1)], &[]);
        tracker.prune_compared();
        tracker.peer.insert((1, 8), 999);

        assert_eq!(tracker.first_mismatch(), Some(8));
    }

    fn tracker(local: &[(u32, u64)], peer: &[(PlayerId, u32, u64)]) -> DesyncTracker {
        DesyncTracker {
            local: local.iter().copied().collect(),
            peer: peer
                .iter()
                .map(|&(player, tick, hash)| ((player, tick), hash))
                .collect(),
        }
    }
}
