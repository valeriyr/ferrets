//! Manages the lifecycle and participants of a running game.

pub mod player_slot;
pub mod player_type;

use bevy_ecs::prelude::*;

use crate::session::player_slot::{PlayerId, PlayerSlot};

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

/// How a finished game ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// `winner` won the game.
    Victory { winner: PlayerId },
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

/// When a session ends on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinishPolicy {
    /// End the game once only one player (or none) still has entities. The
    /// default for an ordinary match.
    #[default]
    LastStanding,
    /// Never end the game automatically — it runs until stopped explicitly.
    /// Suited to sandboxes and tests, where a lone or unpopulated player slot
    /// must not be read as a win.
    Endless,
}

/// Active game session.
///
/// Insert this resource to configure a game before starting the tick loop.
#[derive(Resource, Default)]
pub struct GameSession {
    /// Fixed ticks completed since the session started.
    tick: u32,
    /// Lifecycle state of the session.
    state: SessionState,
    /// Player slots in this session.
    slots: Vec<PlayerSlot>,
    /// The slot controlled by the local client.
    local_player: PlayerId,
    /// When this session ends on its own.
    finish_policy: FinishPolicy,
    /// How the game ended, once it has. `None` while still in progress.
    result: Option<GameResult>,
    /// Whether each slot's player has been dropped (a peer whose frames stopped),
    /// indexed by [`PlayerId`]. A dropped player's input is auto-idled and it is
    /// excluded from the victory check.
    dropped: Vec<bool>,
    /// Whether the session is paused — the tick loop is frozen at the current
    /// tick. Orthogonal to [`SessionState`]; receiving and buffering peer traffic
    /// continues so the game can resume.
    paused: bool,
}

impl GameSession {
    /// Creates a session from the given player slots.
    ///
    /// `local_player` is the [`PlayerId`] controlled by this client, and
    /// `finish_policy` decides when the session ends on its own.
    ///
    /// Panics if the ids are not sorted and contiguous starting from `0` (i.e. `0, 1, 2, …`).
    /// Panics if `local_player` is not in `slots`.
    pub fn new(
        local_player: PlayerId,
        slots: Vec<PlayerSlot>,
        finish_policy: FinishPolicy,
    ) -> Self {
        assert_valid_slots(local_player, &slots);

        Self {
            tick: 0,
            state: SessionState::Pending,
            dropped: vec![false; slots.len()],
            slots,
            local_player,
            finish_policy,
            result: None,
            paused: false,
        }
    }

    /// Replaces the player slots and local player before the game starts. A lobby
    /// builds the running session from its locked configuration this way, mutating
    /// the pending session in place rather than constructing a new one.
    ///
    /// Panics if the session has already started, or if the slot ids are not
    /// contiguous from `0`, or `local_player` is not a valid slot.
    pub fn configure(&mut self, local_player: PlayerId, slots: Vec<PlayerSlot>) {
        assert_eq!(
            self.state,
            SessionState::Pending,
            "configure called after the session started",
        );
        assert_valid_slots(local_player, &slots);
        self.dropped = vec![false; slots.len()];
        self.slots = slots;
        self.local_player = local_player;
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
        matches!(self.state, SessionState::Running | SessionState::Blocked)
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

    /// Returns the slot with the given id, or `None` if out of range.
    pub fn slot(&self, id: PlayerId) -> Option<&PlayerSlot> {
        self.slots.get(id as usize)
    }

    /// Returns the [`PlayerId`] controlled by this client.
    pub fn local_player(&self) -> PlayerId {
        self.local_player
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

    /// Drops `player` from the session — its frames have stopped. From now on its
    /// input is auto-idled and it is excluded from the victory check.
    ///
    /// Panics if `player` was already dropped: a dropped player is filtered out
    /// before a drop is decided, so re-dropping one is a logic bug.
    pub fn drop_player(&mut self, player: PlayerId) {
        let dropped = &mut self.dropped[player as usize];
        assert!(
            !*dropped,
            "player {player} dropped twice — a dropped player must never be re-dropped",
        );
        *dropped = true;
    }

    /// Returns `true` if `player` has been dropped from the session.
    pub fn is_player_dropped(&self, player: PlayerId) -> bool {
        self.dropped.get(player as usize).copied().unwrap_or(false)
    }

    /// The players dropped from the session so far, in ascending id order.
    pub fn dropped_players(&self) -> impl Iterator<Item = PlayerId> {
        self.dropped
            .iter()
            .enumerate()
            .filter_map(|(player, &dropped)| dropped.then_some(player as PlayerId))
    }

    /// The players still active in the session so far, in ascending id order.
    pub fn active_players(&self) -> impl Iterator<Item = PlayerId> {
        self.dropped
            .iter()
            .enumerate()
            .filter_map(|(player, &dropped)| (!dropped).then_some(player as PlayerId))
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
