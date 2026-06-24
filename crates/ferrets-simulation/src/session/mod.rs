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

        Self {
            tick: 0,
            state: SessionState::Pending,
            slots,
            local_player,
            finish_policy,
            result: None,
        }
    }

    pub fn start(&mut self) {
        self.state = SessionState::Running;
    }

    pub fn stop(&mut self) {
        self.state = SessionState::Finished;
    }

    /// Ends the game with the given result and moves the session to
    /// [`Finished`](SessionState::Finished). A no-op once already finished.
    pub fn finish(&mut self, result: GameResult) {
        if self.state != SessionState::Finished {
            self.state = SessionState::Finished;
            self.result = Some(result);
        }
    }

    /// Returns how the game ended, or `None` while it is still in progress.
    pub fn result(&self) -> Option<GameResult> {
        self.result
    }

    /// Sets when this session ends on its own. Configure before starting.
    pub fn set_finish_policy(&mut self, policy: FinishPolicy) {
        self.finish_policy = policy;
    }

    /// Returns when this session ends on its own.
    pub fn finish_policy(&self) -> FinishPolicy {
        self.finish_policy
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

    /// Returns the [`PlayerId`] controlled by this client.
    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }
}
