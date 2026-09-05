//! Fog of war — per-player cell visibility.
//!
//! A deterministic grid, one entry per cell per player, recomputed each tick
//! from the sight of owned entities. Consumers (AI view, combat acquisition,
//! rendering) read it through [`VisibilityGrid::is_visible_to`], which unions a
//! player's own sight with that of its allies.

use bevy_ecs::prelude::*;

use crate::session::{GameSession, player_id::PlayerId};

/// How much of a cell a player currently knows. Ordered least to most known, so
/// a team's combined knowledge of a cell is the maximum over its members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CellVisibility {
    /// Never seen — unknown terrain and contents.
    Unexplored,
    /// Seen before but not in sight now: terrain is remembered, live contents
    /// are stale.
    Explored,
    /// In sight of a friendly source this tick.
    Visible,
}

/// Per-player fog of war, indexed by [`PlayerId`] then `y * width + x`.
#[derive(Resource)]
pub struct VisibilityGrid {
    width: u32,
    height: u32,
    cells: Vec<Vec<CellVisibility>>,
}

impl VisibilityGrid {
    /// Creates an all-unexplored grid for `player_count` players over a
    /// `width × height` map.
    pub fn new(player_count: usize, width: u32, height: u32) -> Self {
        let len = (width * height) as usize;
        Self {
            width,
            height,
            cells: vec![vec![CellVisibility::Unexplored; len]; player_count],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The visibility of `(x, y)` to `player`.
    pub fn get(&self, player: PlayerId, x: u32, y: u32) -> CellVisibility {
        self.assert_player(player);
        self.cells[player as usize][self.index(x, y)]
    }

    /// `player`'s team-combined knowledge of `(x, y)` — the maximum over the
    /// player and its allies. Includes `player` itself, since
    /// [`GameSession::are_allied`] treats a player as allied with itself.
    pub fn visibility_to(
        &self,
        session: &GameSession,
        player: PlayerId,
        x: u32,
        y: u32,
    ) -> CellVisibility {
        let cell = self.index(x, y);
        (0..self.cells.len())
            .filter(|&other| session.are_allied(player, other as PlayerId))
            .map(|other| self.cells[other][cell])
            .max()
            .unwrap_or(CellVisibility::Unexplored)
    }

    /// Whether `player` or any ally currently sees `(x, y)` — the shared team
    /// vision consumers query for gating.
    pub fn is_visible_to(&self, session: &GameSession, player: PlayerId, x: u32, y: u32) -> bool {
        self.visibility_to(session, player, x, y) == CellVisibility::Visible
    }

    /// Demotes every currently-visible cell to explored, before a recompute
    /// re-stamps this tick's sight. `Explored` and `Unexplored` are untouched,
    /// so exploration stays sticky.
    pub fn age(&mut self) {
        for player in &mut self.cells {
            for cell in player.iter_mut() {
                if *cell == CellVisibility::Visible {
                    *cell = CellVisibility::Explored;
                }
            }
        }
    }

    /// Marks `(x, y)` currently visible to `player`.
    pub fn reveal(&mut self, player: PlayerId, x: u32, y: u32) {
        self.assert_player(player);
        let cell = self.index(x, y);
        self.cells[player as usize][cell] = CellVisibility::Visible;
    }

    fn index(&self, x: u32, y: u32) -> usize {
        assert!(
            x < self.width && y < self.height,
            "cell ({x}, {y}) out of range ({}x{})",
            self.width,
            self.height
        );
        (y * self.width + x) as usize
    }

    /// Panics if `player` has no row in this grid.
    fn assert_player(&self, player: PlayerId) {
        assert!(
            (player as usize) < self.cells.len(),
            "player {player} out of range (0..{})",
            self.cells.len()
        );
    }
}
