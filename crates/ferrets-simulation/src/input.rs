//! Player input frames, organized by tick for lockstep execution.

use std::collections::BTreeMap;

/// How many ticks ahead commands are scheduled, giving all peers time to
/// receive them before they must execute. Commands from the UI or network
/// target `session.tick() + SYNC_LATENCY`.
const SYNC_LATENCY: u32 = 2;

use bevy_ecs::prelude::*;

use crate::{command::PlayerCommand, session::player_slot::PlayerId};

/// One player's commands for a single tick.
#[derive(Debug, Clone)]
pub struct PlayerFrame {
    pub player: PlayerId,
    pub tick: u32,
    /// Commands issued this tick. Empty means the player was idle.
    pub commands: Vec<PlayerCommand>,
}

/// All players' commands for one tick, indexed by [`PlayerId`].
pub struct InputFrame {
    slots: Vec<Option<Vec<PlayerCommand>>>,
}

impl InputFrame {
    /// Creates a new empty frame.
    fn new(player_count: usize) -> Self {
        Self {
            slots: vec![None; player_count],
        }
    }

    /// Returns `true` when every player slot has contributed commands for this tick.
    pub fn is_ready(&self) -> bool {
        self.slots.iter().all(|s| s.is_some())
    }

    /// Iterates over `(PlayerId, commands)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (PlayerId, &[PlayerCommand])> {
        self.slots.iter().enumerate().map(|(i, s)| {
            (
                i as PlayerId,
                s.as_deref()
                    .expect("InputFrame::iter called on incomplete frame"),
            )
        })
    }

    /// Returns `true` if commands for `player` have been received.
    fn is_received(&self, player: PlayerId) -> bool {
        self.slots[player as usize].is_some()
    }

    /// Sets the commands for `player`.
    fn set(&mut self, player: PlayerId, commands: Vec<PlayerCommand>) {
        self.slots[player as usize] = Some(commands);
    }

    /// Adds a single command for `player`.
    fn push(&mut self, player: PlayerId, command: PlayerCommand) {
        match &mut self.slots[player as usize] {
            Some(existing) => existing.push(command),
            slot => *slot = Some(vec![command]),
        }
    }
}

/// All received input frames keyed by target tick.
#[derive(Resource)]
pub struct InputFrames {
    player_count: usize,
    frames: BTreeMap<u32, InputFrame>,
}

impl InputFrames {
    /// Creates a new empty input queue.
    pub fn new(player_count: usize) -> Self {
        Self {
            player_count,
            frames: BTreeMap::new(),
        }
    }

    /// Returns the frame for `tick` if all players have contributed, `None` otherwise.
    pub fn get_ready(&self, tick: u32) -> Option<&InputFrame> {
        self.frames.get(&tick).filter(|f| f.is_ready())
    }

    /// Adds a frame from a network peer.
    /// No-op if a frame from that player for that tick was already received.
    pub fn push_player_frame(&mut self, frame: PlayerFrame) {
        let input_frame = self.get_or_insert(frame.tick);

        if !input_frame.is_received(frame.player) {
            input_frame.set(frame.player, frame.commands);
        }
    }

    /// Schedules local player commands for `current_tick + SYNC_LATENCY` and
    /// records an idle frame if no commands were submitted.
    pub fn push_local(
        &mut self,
        player: PlayerId,
        current_tick: u32,
        commands: impl Iterator<Item = PlayerCommand>,
    ) {
        let target_tick = current_tick + SYNC_LATENCY;
        for command in commands {
            self.get_or_insert(target_tick).push(player, command);
        }
        self.ensure_idle(player, target_tick);
    }

    /// Records an empty frame for `player` at `tick` if no input was received yet.
    pub fn ensure_idle(&mut self, player: PlayerId, tick: u32) {
        let is_received = self
            .frames
            .get(&tick)
            .is_some_and(|f| f.is_received(player));

        if !is_received {
            self.get_or_insert(tick).set(player, vec![]);
        }
    }

    /// Returns a mutable reference to the frame for `tick`, creating it if necessary.
    fn get_or_insert(&mut self, tick: u32) -> &mut InputFrame {
        self.frames
            .entry(tick)
            .or_insert_with(|| InputFrame::new(self.player_count))
    }
}
