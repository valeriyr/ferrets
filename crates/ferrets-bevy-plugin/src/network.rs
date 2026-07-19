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

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use ferrets_network::message::control::{ControlMessage, InGameMessage};
use ferrets_network::session::NetSession;
use ferrets_simulation::checksum;
use ferrets_simulation::{
    checksum::CHECKSUM_INTERVAL,
    input::{InputFrames, SYNC_LATENCY},
    session::{
        GameResult, GameSession, authority::Authority, drop_policy::DropPolicy,
        player_slot::PlayerId,
    },
};

use crate::{SimulationSet, session_is_active, session_is_not_paused, session_is_running, systems};

/// How far ahead of the host's tick a pause takes effect. Must exceed the
/// inter-node tick spread (bounded by `SYNC_LATENCY`) so no node has already
/// passed that tick when the authoritative `PauseAt` is sent — then every node
/// reaches it and freezes there. A resume needs no delay: all nodes are already
/// frozen at the same tick, so it applies immediately.
const PAUSE_DELAY: u32 = SYNC_LATENCY * 2;

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

/// A pause/resume scheduled to take effect at an agreed tick, identical on every
/// node so the change is deterministic. Applied (and discarded) by
/// [`net_control`] when each change's tick arrives; the control links are
/// reliable, so a proposal is sent exactly once.
#[derive(Resource, Default)]
pub struct PendingPause(BTreeMap<u32, (PlayerId, bool)>);

impl PendingPause {
    /// Merges a proposal, returning whether it changed what is pending.
    /// Proposals for the same tick resolve identically on every node whatever
    /// their arrival order: the smallest `(player, paused)` wins.
    fn propose(&mut self, tick: u32, player: PlayerId, paused: bool) -> bool {
        match self.0.entry(tick) {
            Entry::Vacant(entry) => {
                entry.insert((player, paused));
                true
            }
            Entry::Occupied(mut entry) => {
                if (player, paused) < *entry.get() {
                    entry.insert((player, paused));
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// The local player's pending pause/resume request (`Some(paused)`), set by the
/// frontend on a pause key. Under host authority [`net_control`] turns it into
/// an authoritative [`PauseAt`](InGameMessage::PauseAt) on the host or forwards
/// it to the host on a client; under peer authority it becomes this node's own
/// tick-stamped proposal on the control mesh. (A local game pauses its
/// session directly and ignores this.)
#[derive(Resource, Default)]
pub struct PauseIntent(pub Option<bool>);

/// Installs the networked session: the `NonSend` session plus its `Send` marker.
/// Call at game start (the lobby does this) so the net systems begin running.
pub fn install_network_session(world: &mut World, session: NetSession) {
    world.insert_non_send_resource(NetworkSession(session));
    world.insert_resource(NetworkActive);
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
        app.init_resource::<DesyncTracker>();
        app.init_resource::<DropConfig>();
        app.init_resource::<BlockedStreak>();
        app.init_resource::<PendingPause>();
        app.init_resource::<PauseIntent>();
        app.init_resource::<Stall>();
        app.init_resource::<DropIntent>();
        app.init_resource::<StallVotes>();
        app.init_resource::<ControlLinks>();
        // Order within the active tick: receive remote frames, then resolve
        // drops, record the local frame (flush_input), broadcast the window,
        // then checksum — all before command_executor consumes the input.
        // `net_receive` and `net_control` run even while paused, so frames and
        // control keep buffering and a resume can be received; everything that
        // advances the simulation is additionally gated on `session_is_not_paused`.
        app.add_systems(
            FixedUpdate,
            net_receive
                .in_set(SimulationSet)
                .before(systems::flush_input)
                .run_if(session_is_active.and(resource_exists::<NetworkActive>)),
        );
        app.add_systems(
            FixedUpdate,
            net_control
                .in_set(SimulationSet)
                .after(net_receive)
                .before(systems::flush_input)
                .run_if(session_is_active.and(resource_exists::<NetworkActive>)),
        );
        app.add_systems(
            FixedUpdate,
            detect_drops
                .in_set(SimulationSet)
                .after(net_control)
                .before(systems::command_executor)
                .run_if(
                    session_is_active
                        .and(resource_exists::<NetworkActive>)
                        .and(session_is_not_paused),
                ),
        );
        app.add_systems(
            FixedUpdate,
            net_broadcast
                .in_set(SimulationSet)
                .after(systems::flush_input)
                .before(systems::command_executor)
                .run_if(
                    session_is_active
                        .and(resource_exists::<NetworkActive>)
                        .and(session_is_not_paused),
                ),
        );
        app.add_systems(
            FixedUpdate,
            net_checksum
                .in_set(SimulationSet)
                .after(net_broadcast)
                .before(systems::command_executor)
                .run_if(
                    session_is_running
                        .and(resource_exists::<NetworkActive>)
                        .and(session_is_not_paused),
                ),
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
    mut intent: ResMut<PauseIntent>,
) {
    let host = net.0.is_host_node();
    let authority = session.authority();
    let tick = session.tick();
    let local = session.local_player();

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
    let mut requests: Vec<bool> = intent.0.take().into_iter().collect();
    let received = net.0.drain_control();
    // Record downed links before reading the messages, so a peer lost this same
    // drain is already known when its relayed vote is judged below.
    links.lost.extend(received.lost.iter().copied());
    for (from, message) in received.messages {
        if let ControlMessage::InGame(message) = message {
            match message {
                InGameMessage::PauseRequest { paused } => requests.push(paused),
                InGameMessage::PauseAt {
                    proposer,
                    tick: effective,
                    paused,
                } => {
                    // An echo of this node's own proposal teaches nobody
                    // anything (the original already went to every link), and
                    // a proposal for a tick already passed is a stale copy of
                    // an applied-and-discarded change — re-learning either
                    // would resurrect it in the apply loop. Legitimate traffic
                    // always targets the present or future: the effective tick
                    // leads the proposer by more than the lockstep skew.
                    if proposer == local || effective < tick {
                        continue;
                    }
                    let news = pending.propose(effective, proposer, paused);
                    if news && authority == Authority::Peers {
                        forward(
                            &mut net,
                            InGameMessage::PauseAt {
                                proposer,
                                tick: effective,
                                paused,
                            },
                        );
                    }
                }
                InGameMessage::DropAt { player, tick: at } => {
                    // The authoritative drop, valid only under host authority
                    // and only from the host's own node. A client cannot drop a
                    // player by sending this, and under peer authority drops
                    // never travel this way — they commit by `StallVote`
                    // consensus in `detect_drops`. A player with a drop already
                    // decided — even one for a tick still ahead — is never
                    // re-dropped.
                    if !matches!(authority, Authority::Host { .. })
                        || !net.0.is_host_peer(from)
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
                    if voter == local {
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
                    let news = votes.0.get(&voter) != Some(&(tick, missing.clone()));
                    if news {
                        votes.0.insert(voter, (tick, missing.clone()));
                        if authority == Authority::Peers {
                            forward(
                                &mut net,
                                InGameMessage::StallVote {
                                    voter,
                                    tick,
                                    missing,
                                },
                            );
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
            // to be gone, so a lost link to it proves no partition.
            let others: Vec<PlayerId> = session
                .occupied_slots()
                .map(|slot| slot.id())
                .filter(|&player| {
                    player != local && !session.is_player_out(player) && net.0.is_networked(player)
                })
                .collect();
            !others.is_empty() && others.iter().all(|player| links.lost.contains(player))
        }
    };
    if unsteerable {
        session.finish(GameResult::Aborted);
        return;
    }

    for paused in requests {
        // Pausing takes effect a margin ahead so every node freezes at the same
        // tick; resuming applies at the current (already frozen) tick.
        let effective = if session.is_paused() {
            tick
        } else {
            tick + PAUSE_DELAY
        };
        let proposal = match authority {
            Authority::Host { .. } if host => InGameMessage::PauseAt {
                proposer: local,
                tick: effective,
                paused,
            },
            Authority::Host { .. } => {
                // A client asks the host to decide; it applies on the resulting
                // PauseAt.
                InGameMessage::PauseRequest { paused }
            }
            // No host to ask: any node proposes directly, and colliding
            // proposals resolve identically everywhere in `propose`.
            Authority::Peers => InGameMessage::PauseAt {
                proposer: local,
                tick: effective,
                paused,
            },
        };
        if let InGameMessage::PauseAt { tick, paused, .. } = proposal {
            let _ = pending.propose(tick, local, paused);
        }
        if let Err(error) = net.0.send_control(&ControlMessage::InGame(proposal)) {
            eprintln!("failed to send pause control: {error}");
        }
    }

    // Apply every change whose tick has arrived, in tick order, then discard
    // it — the control links are reliable, so nothing needs a resend tail.
    let mut applied = false;
    for (_, &(_, paused)) in pending.0.range(..=tick) {
        session.set_paused(paused);
        applied = true;
    }
    if applied {
        // Clear any blocked-streak accrued across the pause boundary so a resume
        // does not immediately trip a drop.
        streak.reset();
        pending.0.retain(|&effective, _| effective > tick);
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
) {
    let received = net.0.drain_received();
    for frame in received.frames {
        frames.push_frame(frame);
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
            if decided {
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
            // included — reports exactly this stall. A voter behind a single
            // broken link still reaches here via the flood; a voter whose
            // control died entirely aborts itself, its frames stop, and it
            // joins the missing set — where its vote was never required.
            let committed = live_others
                .iter()
                .copied()
                .filter(|player| !missing.contains(player))
                .chain(std::iter::once(local))
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
            slot.id() != session.local_player()
                && session.sources_locally(slot, is_host)
                && awaits_frames(session, slot.id())
        })
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
