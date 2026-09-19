//! Fog of war — per-player cell visibility.
//!
//! A deterministic grid, one entry per cell per player, recomputed each tick
//! from the sight of owned entities. Consumers (AI view, combat acquisition,
//! rendering) read it through [`VisibilityGrid::is_visible_to`], which unions a
//! player's own sight with that of its allies.

use bevy_ecs::prelude::*;

use crate::{
    components::location::LocationComponent,
    entity_index::EntityIndex,
    session::{GameSession, ai_vision::AiVision, player_id::PlayerId, player_slot::PlayerSlot},
    simulation_id::SimulationId,
};

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

/// Resolves `id` to an entity `player` may name in a command: interactable —
/// alive and not hidden away inside something
/// ([`EntityIndex::interactable`]) — and in the player's sight (see
/// [`sees`]).
pub fn interactable_to(world: &World, player: PlayerId, id: SimulationId) -> Option<Entity> {
    let entity = world.resource::<EntityIndex>().interactable(world, id)?;
    sees(world, player, entity).then_some(entity)
}

/// Whether `player` may look at `entity` at all: the fog that hides a sprite
/// must hide its stats and refuse orders against it too.
///
/// No ownership shortcut: own and allied entities pass through the same grid (a
/// unit's sight covers the cell it stands on, and team vision is merged), so the
/// grid stays the one truth.
///
/// A scripted player is gated by the vision its seat declares: a fog-limited
/// brain lives under the same rule as a human, an omniscient one legitimately
/// names what fog hides. The seat is session state, so every node (and a
/// replay) resolves its commands identically.
pub fn sees(world: &World, player: PlayerId, entity: Entity) -> bool {
    match world
        .resource::<GameSession>()
        .slot(player)
        .and_then(PlayerSlot::ai_vision)
    {
        // A seat with no brain behind it is a human's, and a human reads the
        // fog like any other.
        None | Some(AiVision::Filtered) => in_sight(world, player, entity),
        Some(AiVision::Omniscient) => true,
    }
}

/// Whether `player` may look at `entity` with the vision given, rather than the
/// one its seat declares — what a view rendered as somebody else would show.
pub fn sees_as(world: &World, player: PlayerId, entity: Entity, vision: AiVision) -> bool {
    match vision {
        AiVision::Omniscient => true,
        AiVision::Filtered => in_sight(world, player, entity),
    }
}

/// Whether `player`'s team's vision covers the cell `entity` stands on.
fn in_sight(world: &World, player: PlayerId, entity: Entity) -> bool {
    let Some(location) = world.entity(entity).get::<LocationComponent>() else {
        return false;
    };
    world.resource::<VisibilityGrid>().is_visible_to(
        world.resource::<GameSession>(),
        player,
        location.position.x.to_num::<u32>(),
        location.position.y.to_num::<u32>(),
    )
}
