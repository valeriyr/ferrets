//! Manages the lifecycle and participants of a running game.

pub mod ai_hosting;
pub mod authority;
pub mod drop_policy;
pub mod finish_policy;
pub mod player_slot;
pub mod player_type;

use crate::session::ai_hosting::AiHosting;
use crate::session::authority::Authority;
use crate::session::drop_policy::DropPolicy;
use crate::session::finish_policy::FinishPolicy;
use crate::session::player_slot::{Participation, PlayerId, PlayerSlot, TeamId};
use crate::session::player_type::PlayerType;
use bevy_ecs::prelude::*;

/// Lifecycle state of the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionState {
    /// Configured but the tick loop has not started yet.
    #[default]
    Pending,
    /// Tick loop is running normally.
    Running,
    /// Waiting for peers to deliver their commands for the current tick.
    Blocked,
    /// The game has finished (victory, defeat, or disconnect).
    Finished,
}

/// The side that won a [`Victory`](GameResult::Victory): a whole team, or a lone
/// player on no team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    Team(TeamId),
    Player(PlayerId),
}

/// How a finished game ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// `winner` won the game.
    Victory { winner: Winner },
    /// The local player was defeated — a scenario's failure condition was met, or
    /// their team was wiped out in a last-standing match. Not how a winner is
    /// chosen, and not necessarily identical on every node: while other teams
    /// fight on there is no shared result yet, so this is what tells this node's
    /// player they are out.
    Defeat,
    /// The game ended with no winner.
    Draw,
    /// Peers diverged at `tick`; the game cannot continue deterministically.
    Desynchronization { tick: u32 },
    /// The local node can no longer participate — it lost the host, or is
    /// partitioned from every remaining peer. A *local* outcome (the other peers
    /// drop this node and continue), not a shared result, and never the way a
    /// winner is decided.
    Aborted,
}

/// Active game session.
///
/// Insert this resource to configure a game before starting the tick loop.
#[derive(Resource)]
pub struct GameSession {
    /// Fixed ticks completed since the session started.
    tick: u32,
    /// Lifecycle state of the session.
    state: SessionState,
    /// Player slots in this session.
    slots: Vec<PlayerSlot>,
    /// The map this session plays on, by name — the session holds the
    /// agreement, not the map itself.
    map: String,
    /// The slot controlled by the local client.
    local_player: PlayerId,
    /// Who resolves session-level decisions (drops, pauses), and the
    /// host-dependent choices that come with a host.
    authority: Authority,
    /// When a deciding node turns a stall into a drop.
    drop_policy: DropPolicy,
    /// When this session ends on its own.
    finish_policy: FinishPolicy,
    /// How the game ended, once it has. `None` while still in progress.
    result: Option<GameResult>,
    /// The tick from which each slot's player contributes no further input (a
    /// peer whose frames stopped), indexed by [`PlayerId`]. `None` while no
    /// drop has been decided; a decided drop may name a tick still ahead, in
    /// effect only once that tick arrives.
    dropped: Vec<Option<u32>>,
    /// The tick from which each slot's player is out of the game — defeated by
    /// the finish rule, contributing no further input — indexed by
    /// [`PlayerId`]. `None` while the player is still in the game. A separate
    /// exclusion from `dropped`: a defeat is derived from the simulation
    /// identically on every node, while a drop is a networking decision, and
    /// one player may end up marked with both.
    eliminated: Vec<Option<u32>>,
    /// Whether the session is paused — the tick loop is frozen at the current
    /// tick. Orthogonal to [`SessionState`]; receiving and buffering peer traffic
    /// continues so the game can resume.
    paused: bool,
}

impl GameSession {
    /// Creates a session from an already-decided configuration: the slots and
    /// session-level choices a lobby would otherwise install via
    /// [`configure`](Self::configure).
    ///
    /// `local_player` is the [`PlayerId`] controlled by this client; the
    /// remaining choices are the session-level agreement, each valid in any
    /// combination by construction.
    ///
    /// Panics if the ids are not sorted and contiguous starting from `0` (i.e. `0, 1, 2, …`).
    /// Panics if `local_player` is not in `slots`.
    pub fn configured(
        local_player: PlayerId,
        slots: Vec<PlayerSlot>,
        map: impl Into<String>,
        authority: Authority,
        drop_policy: DropPolicy,
        finish_policy: FinishPolicy,
    ) -> Self {
        assert_valid_slots(local_player, &slots);
        Self::new(
            local_player,
            slots,
            map,
            authority,
            drop_policy,
            finish_policy,
        )
    }

    /// The inert pre-configuration placeholder: a game inserts the resource
    /// before its lobby has decided anything, then
    /// [`configure`](Self::configure) installs the real slots and choices.
    /// Every value here is fabricated and unread — the session has no slots
    /// and stays [`Pending`](SessionState::Pending), so nothing runs until it
    /// is configured and started.
    pub fn pending() -> Self {
        Self::new(
            0,
            Vec::new(),
            String::new(),
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::LastStanding,
        )
    }

    /// Replaces the player slots, local player, and session-level choices
    /// before the game starts. A lobby builds the running session from its
    /// locked configuration this way, mutating the pending session in place
    /// rather than constructing a new one.
    ///
    /// Panics if the session has already started, or if the slot ids are not
    /// contiguous from `0`, or `local_player` is not a valid slot.
    pub fn configure(
        &mut self,
        local_player: PlayerId,
        slots: Vec<PlayerSlot>,
        map: impl Into<String>,
        authority: Authority,
        drop_policy: DropPolicy,
        finish_policy: FinishPolicy,
    ) {
        assert_eq!(
            self.state,
            SessionState::Pending,
            "configure called after the session started",
        );
        assert_valid_slots(local_player, &slots);
        self.dropped = vec![None; slots.len()];
        self.eliminated = vec![None; slots.len()];
        self.slots = slots;
        self.map = map.into();
        self.local_player = local_player;
        self.authority = authority;
        self.drop_policy = drop_policy;
        self.finish_policy = finish_policy;
    }

    pub fn start(&mut self) {
        self.state = SessionState::Running;
    }

    /// Ends the game with the given result and moves the session to
    /// [`Finished`](SessionState::Finished). A no-op once already finished.
    pub fn finish(&mut self, result: GameResult) {
        if self.state != SessionState::Finished {
            self.state = SessionState::Finished;
            self.result = Some(result);
        }
    }

    /// Blocks the tick loop (waiting for input) or resumes it. Only toggles
    /// between [`Running`](SessionState::Running) and
    /// [`Blocked`](SessionState::Blocked); a `Pending` or `Finished` session is
    /// left unchanged.
    pub fn set_blocked(&mut self, blocked: bool) {
        if self.is_active() {
            self.state = if blocked {
                SessionState::Blocked
            } else {
                SessionState::Running
            };
        }
    }

    /// Returns `true` when the tick loop is advancing normally.
    pub fn is_running(&self) -> bool {
        self.state == SessionState::Running
    }

    /// Returns `true` when the tick loop is paused waiting for peer input.
    pub fn is_blocked(&self) -> bool {
        self.state == SessionState::Blocked
    }

    /// Returns `true` when the session is running or blocked.
    ///
    /// Use this to gate systems that must still run while blocked (e.g. network input collection).
    pub fn is_active(&self) -> bool {
        match self.state {
            SessionState::Running | SessionState::Blocked => true,
            SessionState::Pending | SessionState::Finished => false,
        }
    }

    /// Pauses or resumes the session. While paused the tick loop is frozen; peer
    /// traffic is still received and buffered so the game can resume cleanly.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Returns `true` while the session is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Returns how AI player input is computed.
    pub fn ai_hosting(&self) -> AiHosting {
        self.authority.ai_hosting()
    }

    /// Returns who resolves session-level decisions.
    pub fn authority(&self) -> Authority {
        self.authority
    }

    /// Returns when a deciding node turns a stall into a drop.
    pub fn drop_policy(&self) -> DropPolicy {
        self.drop_policy
    }

    /// Sets when a deciding node turns a stall into a drop. Read live each time
    /// a stall is resolved, so it takes effect whenever it is set.
    pub fn set_drop_policy(&mut self, policy: DropPolicy) {
        self.drop_policy = policy;
    }

    /// Returns `true` when this node is responsible for producing input frames
    /// for `slot`. `is_host` says whether this node is the session host; a
    /// local game's single node is its own host.
    ///
    /// Unoccupied slots are sourced by nobody — there is no player, so no
    /// tick requires their input. A human slot is sourced by the node the
    /// human plays on, and an AI slot according to the session's
    /// [`AiHosting`].
    pub fn sources_locally(&self, slot: &PlayerSlot, is_host: bool) -> bool {
        match slot.player_type() {
            None => false,
            Some(PlayerType::Human) => slot.id() == self.local_player,
            Some(PlayerType::Ai) => match self.ai_hosting() {
                AiHosting::Replicated => true,
                AiHosting::Host => is_host,
            },
        }
    }

    /// Sets when this session ends on its own. Configure before starting.
    pub fn set_finish_policy(&mut self, policy: FinishPolicy) {
        self.finish_policy = policy;
    }

    /// Returns when this session ends on its own.
    pub fn finish_policy(&self) -> FinishPolicy {
        self.finish_policy
    }

    /// Returns how the game ended, or `None` while it is still in progress.
    pub fn result(&self) -> Option<GameResult> {
        self.result
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Increments the tick counter. Called by the tick-counter system each tick.
    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn slots(&self) -> &[PlayerSlot] {
        &self.slots
    }

    /// Returns every occupied slot, of any participation.
    pub fn occupied_slots(&self) -> impl Iterator<Item = &PlayerSlot> {
        self.slots
            .iter()
            .filter(|slot| slot.participation().is_some())
    }

    /// Returns every slot occupied as [`Participation::Player`].
    pub fn player_slots(&self) -> impl Iterator<Item = &PlayerSlot> {
        self.slots
            .iter()
            .filter(|slot| slot.participation() == Some(Participation::Player))
    }

    /// Returns every slot occupied as [`Participation::Environment`].
    pub fn environment_slots(&self) -> impl Iterator<Item = &PlayerSlot> {
        self.slots
            .iter()
            .filter(|slot| slot.participation() == Some(Participation::Environment))
    }

    /// Returns `true` if `player`'s slot is occupied as
    /// [`Participation::Environment`].
    pub fn is_environment_slot(&self, player: PlayerId) -> bool {
        self.slot(player)
            .is_some_and(|slot| slot.participation() == Some(Participation::Environment))
    }

    /// The map this session plays on, by name.
    pub fn map(&self) -> &str {
        &self.map
    }

    /// Returns the slot with the given id, or `None` if out of range.
    pub fn slot(&self, id: PlayerId) -> Option<&PlayerSlot> {
        self.slots.get(id as usize)
    }

    /// Returns the [`PlayerId`] controlled by this client.
    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }

    /// Returns `true` when `a` and `b` are allies: the same player, or two
    /// players sharing a team. Players on no team (and unknown slot ids) are
    /// allied with no one but themselves.
    pub fn are_allied(&self, a: PlayerId, b: PlayerId) -> bool {
        if a == b {
            return true;
        }
        match (
            self.slot(a).and_then(PlayerSlot::team),
            self.slot(b).and_then(PlayerSlot::team),
        ) {
            (Some(team_a), Some(team_b)) => team_a == team_b,
            _ => false,
        }
    }

    /// Returns `true` when `player` is on the winning side named by `winner`: the
    /// same lone player, or a member of the winning team.
    pub fn is_winner(&self, player: PlayerId, winner: Winner) -> bool {
        match winner {
            Winner::Team(team) => self.slot(player).and_then(PlayerSlot::team) == Some(team),
            Winner::Player(id) => player == id,
        }
    }

    /// Sets the race the given player plays. Lets a menu pick the race after the
    /// session is built.
    ///
    /// Panics if `player` is not a valid slot id.
    pub fn set_race(&mut self, player: PlayerId, race: impl Into<String>) {
        self.slots
            .get_mut(player as usize)
            .expect("set_race called with an unknown player id")
            .set_race(race);
    }

    /// Drops `player` from the session: from tick `from` on it contributes no
    /// input and it is excluded from the victory check; ticks before `from`
    /// keep the input it already contributed. `from` must be agreed by every
    /// node (it decides which of the player's final frames still execute), so
    /// it is passed in rather than read from the local tick — the current
    /// (not yet executed) tick or any tick ahead of it.
    ///
    /// Panics if `from` lies behind the current tick: executed ticks keep the
    /// input they ran with, so a drop never rewrites one. Panics if `player`
    /// was already dropped: a dropped player is filtered out before a drop is
    /// decided, so re-dropping one is a logic bug.
    pub fn drop_player(&mut self, player: PlayerId, from: u32) {
        assert!(
            from >= self.tick,
            "player {player} dropped from tick {from} behind the current tick {} — a drop \
             applies only from the current tick on",
            self.tick,
        );
        let dropped = &mut self.dropped[player as usize];
        assert!(
            dropped.is_none(),
            "player {player} dropped twice — a dropped player must never be re-dropped",
        );
        *dropped = Some(from);
    }

    /// The tick from which `player` contributes no further input, or `None`
    /// while no drop has been decided. Present as soon as the drop is decided,
    /// even when the named tick still lies ahead.
    pub fn drop_tick(&self, player: PlayerId) -> Option<u32> {
        self.dropped.get(player as usize).copied().flatten()
    }

    /// Returns `true` if `player` is dropped as of the current tick. A drop
    /// decided for a tick still ahead is pending, not yet in effect: the
    /// player keeps playing — and keeps being required — until that tick
    /// arrives ([`drop_tick`](Self::drop_tick) reports the pending mark).
    pub fn is_player_dropped(&self, player: PlayerId) -> bool {
        self.mark_arrived(&self.dropped, player)
    }

    /// Eliminates `player` from the session: from tick `from` on it contributes
    /// no input. A defeat is derived from the simulation, so every node marks
    /// it at the same tick on its own — no message carries it. Independent of
    /// [`drop_player`](Self::drop_player): a defeated player whose node also
    /// vanishes may carry both marks.
    ///
    /// Panics if `from` is not ahead of the current tick: the present tick
    /// executed with the player's input on every node and must keep requiring
    /// it, so an elimination applies only to future ticks. Panics if `player`
    /// was already eliminated: a defeat is detected once, so re-eliminating a
    /// player is a logic bug.
    pub fn eliminate_player(&mut self, player: PlayerId, from: u32) {
        assert!(
            from > self.tick,
            "player {player} eliminated from tick {from}, not ahead of the current tick {} — an \
             elimination applies only to future ticks",
            self.tick,
        );
        let eliminated = &mut self.eliminated[player as usize];
        assert!(
            eliminated.is_none(),
            "player {player} eliminated twice — an eliminated player must never be re-eliminated",
        );
        *eliminated = Some(from);
    }

    /// Returns `true` if `player` is eliminated as of the current tick. An
    /// elimination is always marked ahead of the current tick, so this turns
    /// `true` exactly when the elimination takes effect.
    pub fn is_player_eliminated(&self, player: PlayerId) -> bool {
        self.mark_arrived(&self.eliminated, player)
    }

    /// Returns `true` if `player` is out of the game as of the current tick —
    /// dropped or eliminated — and so contributes no further input.
    pub fn is_player_out(&self, player: PlayerId) -> bool {
        self.is_player_dropped(player) || self.is_player_eliminated(player)
    }

    /// Whether the tick `player` is marked with in `marks` has arrived — the
    /// exclusion is in effect, not merely decided for a tick still ahead.
    fn mark_arrived(&self, marks: &[Option<u32>], player: PlayerId) -> bool {
        marks
            .get(player as usize)
            .copied()
            .flatten()
            .is_some_and(|from| from <= self.tick)
    }

    /// Returns `true` when `tick` needs `player`'s input before it can
    /// execute: the slot is occupied and neither the player's drop nor its
    /// elimination has taken effect by that tick. The exclusion ticks are
    /// derived or agreed identically on every node, so every node answers
    /// this identically for any tick — past, present, or future.
    pub fn is_player_required(&self, player: PlayerId, tick: u32) -> bool {
        let occupied = self
            .slot(player)
            .is_some_and(|slot| slot.player_type().is_some());
        let live_at = |marks: &[Option<u32>]| marks[player as usize].is_none_or(|from| tick < from);
        occupied && live_at(&self.dropped) && live_at(&self.eliminated)
    }

    /// The players whose input `tick` needs before it can execute, in ascending
    /// id order: every occupied slot whose player was still in the game at that
    /// tick. A dropped or eliminated player stops counting at its drop or
    /// elimination tick, so a past tick keeps requiring — and a recording of it
    /// keeps carrying — the input the player contributed while it played.
    pub fn required_players(&self, tick: u32) -> Vec<PlayerId> {
        self.slots
            .iter()
            .map(|slot| slot.id())
            .filter(|&player| self.is_player_required(player, tick))
            .collect()
    }

    /// The players with a drop decided so far — in effect or naming a tick
    /// still ahead — in ascending id order.
    pub fn dropped_players(&self) -> impl Iterator<Item = PlayerId> {
        self.dropped
            .iter()
            .enumerate()
            .filter_map(|(player, dropped)| dropped.map(|_| player as PlayerId))
    }

    /// The players still in the game as of the current tick — neither dropped
    /// nor eliminated — in ascending id order.
    pub fn active_players(&self) -> impl Iterator<Item = PlayerId> {
        (0..self.dropped.len())
            .map(|player| player as PlayerId)
            .filter(|&player| !self.is_player_out(player))
    }

    /// The one place the fields are initialized. Validation belongs to the
    /// public constructors: [`configured`](Self::configured) demands a
    /// coherent slot list, while [`pending`](Self::pending) is deliberately
    /// slotless.
    fn new(
        local_player: PlayerId,
        slots: Vec<PlayerSlot>,
        map: impl Into<String>,
        authority: Authority,
        drop_policy: DropPolicy,
        finish_policy: FinishPolicy,
    ) -> Self {
        Self {
            tick: 0,
            state: SessionState::Pending,
            dropped: vec![None; slots.len()],
            eliminated: vec![None; slots.len()],
            slots,
            map: map.into(),
            local_player,
            authority,
            drop_policy,
            finish_policy,
            result: None,
            paused: false,
        }
    }
}

/// Asserts the slot ids are contiguous from `0` and `local_player` is one of them.
fn assert_valid_slots(local_player: PlayerId, slots: &[PlayerSlot]) {
    for (expected, slot) in slots.iter().enumerate() {
        assert_eq!(
            slot.id() as usize,
            expected,
            "slot ids must be contiguous starting from 0, expected {expected} got {}",
            slot.id()
        );
    }
    assert!(
        (local_player as usize) < slots.len(),
        "local_player {local_player} is not in slots (len {})",
        slots.len()
    );
}
