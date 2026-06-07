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
}

impl GameSession {
    /// Creates a session from the given player slots.
    ///
    /// `local_player` is the [`PlayerId`] controlled by this client.
    ///
    /// Panics if the ids are not sorted and contiguous starting from `0` (i.e. `0, 1, 2, …`).
    /// Panics if `local_player` is not in `slots`.
    pub fn new(local_player: PlayerId, slots: Vec<PlayerSlot>) -> Self {
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
        }
    }

    pub fn start(&mut self) {
        self.state = SessionState::Running;
    }

    pub fn stop(&mut self) {
        self.state = SessionState::Finished;
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

    /// Returns the [`PlayerId`] controlled by this client.
    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }
}
