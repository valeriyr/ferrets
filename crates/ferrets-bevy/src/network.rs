//! Bevy wiring for lockstep networking.
//!
//! Bridges the [`NetSession`] to the simulation each `FixedUpdate`: [`net_receive`]
//! injects remote players' frames, [`flush_input`](crate::flush_input) records the
//! local frame, [`net_broadcast`] (re)broadcasts the frame window, [`net_checksum`]
//! exchanges per-tick state hashes to catch desyncs, and [`net_pause_control`]
//! applies tick-aligned pauses. Add it alongside
//! [`SimulationPlugin`](crate::SimulationPlugin) for every game: the systems run
//! only once a [`NetworkSession`] is installed, which a local game never does.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_network::message::control::{ControlMessage, InGameMessage};
use ferrets_network::session::NetSession;
use ferrets_simulation::checksum;
use ferrets_simulation::{
    checksum::CHECKSUM_INTERVAL,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::{GameResult, GameSession, player_slot::PlayerId},
};

use crate::{SimulationSet, session_is_active, session_is_not_paused, session_is_running, systems};

/// How far ahead of the host's tick a pause takes effect. Must exceed the
/// inter-node tick spread (bounded by `SYNC_LATENCY`) so no node has already
/// passed that tick when the authoritative `PauseAt` is sent — then every node
/// reaches it and freezes there. A resume needs no delay: all nodes are already
/// frozen at the same tick, so it applies immediately.
const PAUSE_DELAY: u32 = SYNC_LATENCY * 2;

/// How long the tick may stay blocked on a player before that player is dropped
/// — the reconnection grace window, measured in blocked `FixedUpdate` steps (not
/// wall-clock, so it is testable and only gates *when* a node acts). Generous
/// enough to ride out a link blip; the freeze lasts this long on a genuine drop.
#[derive(Resource)]
pub struct DropConfig {
    pub timeout_steps: u32,
}

impl Default for DropConfig {
    fn default() -> Self {
        // ~4 s at 20 Hz.
        Self { timeout_steps: 80 }
    }
}

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
/// node so the change is deterministic. Applied by [`net_pause_control`] when the
/// tick arrives.
#[derive(Resource, Default)]
pub struct PendingPause(Option<(u32, bool)>);

/// The local player's pending pause/resume request (`Some(paused)`), set by the
/// frontend on a pause key. [`net_pause_control`] turns it into an authoritative
/// [`PauseAt`](InGameMessage::PauseAt) on the host, or forwards it to the host on
/// a client. (A local game pauses its session directly and ignores this.)
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
        // Order within the active tick: receive remote frames, then resolve drops
        // and auto-idle dropped slots, record the local frame (flush_input),
        // broadcast the window, then checksum — all before command_executor
        // consumes the input.
        // `net_receive` and `net_pause_control` run even while paused, so frames
        // and control keep buffering and a resume can be received; everything that
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
            net_pause_control
                .in_set(SimulationSet)
                .after(net_receive)
                .before(systems::flush_input)
                .run_if(session_is_active.and(resource_exists::<NetworkActive>)),
        );
        app.add_systems(
            FixedUpdate,
            (detect_drops, auto_idle_dropped)
                .chain()
                .in_set(SimulationSet)
                .after(net_receive)
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

/// Receives in-game control and applies pauses deterministically.
///
/// A client's [`PauseRequest`](InGameMessage::PauseRequest) is forwarded by the
/// host as an authoritative [`PauseAt`](InGameMessage::PauseAt) at a tick far
/// enough ahead that every node learns of it first; each node then pauses/resumes
/// exactly when its tick reaches that value, so all freeze at the same tick. Runs
/// while paused (it must still receive the resume).
pub fn net_pause_control(
    mut net: NonSendMut<NetworkSession>,
    mut session: ResMut<GameSession>,
    mut pending: ResMut<PendingPause>,
    mut streak: ResMut<BlockedStreak>,
    mut intent: ResMut<PauseIntent>,
) {
    let host = net.0.is_control_host();
    let tick = session.tick();

    // Pause/resume requests to act on: the local player's, plus (on the host) any
    // a client sent over the wire. A `PauseAt` is the host's decision — apply it.
    let mut requests: Vec<bool> = intent.0.take().into_iter().collect();
    for message in net.0.drain_control() {
        if let ControlMessage::InGame(message) = message {
            match message {
                InGameMessage::PauseRequest { paused } => requests.push(paused),
                InGameMessage::PauseAt { tick, paused } => pending.0 = Some((tick, paused)),
            }
        }
    }

    for paused in requests {
        let message = if host {
            // Authoritative: pausing takes effect a margin ahead so every node
            // freezes at the same tick; resuming applies at the current (already
            // frozen) tick. The host also schedules its own pending change.
            let effective = if session.is_paused() {
                tick
            } else {
                tick + PAUSE_DELAY
            };
            pending.0 = Some((effective, paused));
            InGameMessage::PauseAt {
                tick: effective,
                paused,
            }
        } else {
            // A client asks the host to decide; it applies on the resulting PauseAt.
            InGameMessage::PauseRequest { paused }
        };
        if let Err(error) = net.0.send_control(&ControlMessage::InGame(message)) {
            eprintln!("failed to send pause control: {error}");
        }
    }

    if let Some((effective, paused)) = pending.0
        && session.tick() >= effective
    {
        session.set_paused(paused);
        // Clear any blocked-streak accrued across the pause boundary so a resume
        // does not immediately trip a drop.
        streak.reset();
        pending.0 = None;
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

/// Drops players whose frames have stopped, once the tick has been blocked on
/// them for longer than the grace window — or aborts locally if *every* other
/// live player is missing (this node is partitioned).
///
/// Deterministic in effect: a truly-gone player produces no frame for the blocked
/// tick `B` on any node, so every still-connected node computes the same missing
/// set at the same `B` and drops it; the grace counter only gates *when*. (If a
/// frame does arrive, `command_executor` advances the tick and the streak resets
/// before the timeout — the automatic veto against dropping a merely-laggy player.)
pub fn detect_drops(
    frames: Res<InputFrames>,
    config: Res<DropConfig>,
    mut streak: ResMut<BlockedStreak>,
    mut session: ResMut<GameSession>,
) {
    if !session.is_blocked() {
        streak.reset();
        return;
    }

    let tick = session.tick();
    if streak.tick == Some(tick) {
        streak.steps += 1;
    } else {
        streak.tick = Some(tick);
        streak.steps = 1;
    }
    if streak.steps < config.timeout_steps {
        return;
    }

    let local = session.local_player();
    let live_others: Vec<PlayerId> = session
        .slots()
        .iter()
        .filter(|slot| slot.player_type().is_some())
        .map(|slot| slot.id())
        .filter(|&player| player != local && !session.is_player_dropped(player))
        .collect();
    let missing: Vec<PlayerId> = live_others
        .iter()
        .copied()
        .filter(|&player| !frames.has_frame(player, tick))
        .collect();

    if missing.is_empty() {
        return;
    }
    if missing.len() == live_others.len() {
        // Missing everyone reachable → this node is the one cut off; it cannot
        // determine a global tail, so it ends locally rather than dropping all.
        session.finish(GameResult::Aborted);
    } else {
        for player in missing {
            session.drop_player(player);
        }
    }
    streak.reset();
}

/// Supplies an idle frame for each dropped player at the current tick, so the
/// gone slot no longer blocks lockstep. Runs after [`net_receive`] (a real frame
/// wins, first-write) and before `command_executor`; deterministic because the
/// dropped set and tick are identical on every node.
pub fn auto_idle_dropped(mut frames: ResMut<InputFrames>, session: Res<GameSession>) {
    let tick = session.tick();
    for player in session.dropped_players() {
        frames.push_frame(PlayerFrame::idle(player, tick));
    }
}

/// (Re)broadcasts the frame window read from `InputFrames` — the single source of
/// truth — around the current tick.
///
/// Selects only what belongs on the wire: network-backed players' frames (which
/// players those are depends on the session's AI hosting mode — a replicated
/// AI is computed on every node and never relayed), never dropped players'
/// (their idle is synthesized locally everywhere — broadcasting it could race a
/// late real frame), and on a non-relay node only the players this node sources
/// (its own input, plus any AIs it computes for the others). Re-reading the
/// `[tick-SYNC_LATENCY, tick+SYNC_LATENCY]` window each tick is the redundancy
/// resend; idempotent `push_frame` makes duplicates harmless.
pub fn net_broadcast(
    mut net: NonSendMut<NetworkSession>,
    frames: Res<InputFrames>,
    session: Res<GameSession>,
) {
    let tick = session.tick();
    let relays = net.0.relays();
    let is_host = net.0.is_control_host();

    let mut window = frames.frames_in_range(tick.saturating_sub(SYNC_LATENCY), tick + SYNC_LATENCY);
    window.retain(|frame| {
        let sourced = session
            .slot(frame.player)
            .is_some_and(|slot| session.sources_locally(slot, is_host));
        net.0.is_networked(frame.player)
            && !session.is_player_dropped(frame.player)
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
