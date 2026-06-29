//! Player input frames, organized by tick for lockstep execution.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{command::PlayerCommand, session::player_slot::PlayerId};

/// How many ticks ahead commands are scheduled, giving all peers time to
/// receive them before they must execute. Commands from the UI or network
/// target `session.tick() + SYNC_LATENCY`.
pub const SYNC_LATENCY: u32 = 2;

/// One player's commands for a single tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerFrame {
    pub player: PlayerId,
    pub tick: u32,
    /// Commands issued this tick. Empty means the player was idle.
    pub commands: Vec<PlayerCommand>,
}

impl PlayerFrame {
    /// A frame carrying no commands — the player did nothing this tick.
    pub fn idle(player: PlayerId, tick: u32) -> Self {
        Self {
            player,
            tick,
            commands: Vec::new(),
        }
    }
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

    /// Returns `true` if `player` has contributed a frame for this tick.
    fn has(&self, player: PlayerId) -> bool {
        self.slots[player as usize].is_some()
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

    /// Records `commands` for `player`, the first time wins. A repeat for an
    /// already-recorded slot must be byte-identical (a redundant copy); a
    /// *differing* one is a determinism bug, so it is asserted in debug builds and
    /// ignored in release (keeping the committed input immutable).
    fn record(&mut self, player: PlayerId, commands: Vec<PlayerCommand>) {
        let slot = &mut self.slots[player as usize];
        match slot {
            Some(existing) => debug_assert!(
                *existing == commands,
                "conflicting input for player {player}: input is immutable once recorded",
            ),
            None => *slot = Some(commands),
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

    /// Returns `true` if `player` has a frame recorded for `tick` — used to find
    /// which slot is holding up a blocked tick.
    pub fn has_frame(&self, player: PlayerId, tick: u32) -> bool {
        self.frames.get(&tick).is_some_and(|f| f.has(player))
    }

    /// Clones every recorded frame whose tick is in `[from, to]` (inclusive),
    /// ascending by tick then player. The networking layer reads this window to
    /// (re)broadcast; it then selects which players' frames actually go on the
    /// wire (the queue itself holds locally-synthesized fills too).
    pub fn frames_in_range(&self, from: u32, to: u32) -> Vec<PlayerFrame> {
        let mut frames = Vec::new();
        for (&tick, frame) in self.frames.range(from..=to) {
            for (player, slot) in frame.slots.iter().enumerate() {
                if let Some(commands) = slot {
                    frames.push(PlayerFrame {
                        player: player as PlayerId,
                        tick,
                        commands: commands.clone(),
                    });
                }
            }
        }
        frames
    }

    /// Records a player's frame for its tick — the single way input enters the
    /// queue. Idempotent: re-recording the same `(player, tick)` is a no-op, so
    /// redundant copies (the resend window, relay hops) are safe; for an idle
    /// player pass [`PlayerFrame::idle`]. A *differing* repeat is a determinism
    /// bug — asserted in debug builds, ignored in release.
    pub fn push_frame(&mut self, frame: PlayerFrame) {
        self.get_or_insert(frame.tick)
            .record(frame.player, frame.commands);
    }

    /// Returns a mutable reference to the frame for `tick`, creating it if necessary.
    fn get_or_insert(&mut self, tick: u32) -> &mut InputFrame {
        self.frames
            .entry(tick)
            .or_insert_with(|| InputFrame::new(self.player_count))
    }
}
