//! Fog of war and sighting: the per-player cell visibility grid, and what a
//! side makes of an entity given that grid, the entity's concealment and the
//! side's detection.

use bevy_ecs::prelude::*;
use ferrets_content::{affiliation::Affiliation, entity_type_def::EntityTypeDef};

use crate::{
    components::{
        concealed::ConcealedComponent, hidden::HiddenComponent, location::LocationComponent, owner,
    },
    entity_def,
    entity_index::EntityIndex,
    fields,
    session::{
        GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
        player_slot::PlayerSlot,
    },
    simulation_id::SimulationId,
    watches,
};

/// What one side makes of one entity this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sighting {
    /// Nothing: the side's sight reaches no cell the entity stands on.
    Unseen,
    /// A presence and no more: the side's sight reaches the cell, but the
    /// entity is concealed and the side's detection does not reach it.
    Glimpsed,
    /// The entity itself.
    Seen,
}

impl Sighting {
    /// Whether the sighting is the entity itself.
    pub fn is_seen(self) -> bool {
        match self {
            Sighting::Seen => true,
            Sighting::Unseen | Sighting::Glimpsed => false,
        }
    }
}

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

/// Whose senses a sighting is judged by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Senses {
    /// Ordinary sight and ordinary detectors: the rule every side acts under,
    /// whatever its seat declares.
    Ordinary,
    /// The ones the player's seat declares, privilege included. A free seat
    /// declares none, and makes out nothing.
    SeatDeclared,
    /// A pair named outright.
    Given(AiVision, AiDetection),
}

/// Resolves `id` to an entity `player` may name in a command: interactable —
/// alive and not hidden away inside something
/// ([`EntityIndex::interactable`]) — and sighted by what the player's seat
/// declares, so a seat granted a privilege may name what it reaches.
pub fn interactable_to(world: &World, player: PlayerId, id: SimulationId) -> Option<Entity> {
    let entity = world.resource::<EntityIndex>().interactable(world, id)?;
    sees(world, player, entity, Senses::SeatDeclared).then_some(entity)
}

/// Resolves `id` to remains `player` may name in a command, the way
/// [`interactable_to`] resolves a standing entity.
pub fn remains_interactable_to(
    world: &World,
    player: PlayerId,
    id: SimulationId,
) -> Option<Entity> {
    let remains = world.resource::<EntityIndex>().remains(id)?;
    sees(world, player, remains, Senses::SeatDeclared).then_some(remains)
}

/// Whether `player` makes `entity` out in full, by `senses`.
pub fn sees(world: &World, player: PlayerId, entity: Entity, senses: Senses) -> bool {
    sighting(world, player, entity, senses).is_seen()
}

/// [`sees`] for an `entity` whose type `def` the caller already holds.
pub fn sees_of(
    world: &World,
    player: PlayerId,
    def: &EntityTypeDef,
    entity: Entity,
    senses: Senses,
) -> bool {
    sighting_of(world, player, def, entity, senses).is_seen()
}

/// What `player` makes of `entity` this tick, by `senses`.
///
/// An entity off the map — aboard, inside a site or a mine — is unseen by
/// everyone, its own side included, whatever the vision; a side sees its own
/// and its allies' on-map entities wherever they stand. Sight, detection and a
/// field's concealment each answer over every cell the entity occupies, and any
/// one of them settles the question. Omniscient vision lifts the fog and
/// nothing else: a concealed entity still needs detection to be made out.
/// A free seat, or a player id with no seat, makes out nothing.
pub fn sighting(world: &World, player: PlayerId, entity: Entity, senses: Senses) -> Sighting {
    sighting_of(world, player, entity_def::of(world, entity), entity, senses)
}

/// [`sighting`] for an `entity` whose type `def` the caller already holds.
pub fn sighting_of(
    world: &World,
    player: PlayerId,
    def: &EntityTypeDef,
    entity: Entity,
    senses: Senses,
) -> Sighting {
    let session = world.resource::<GameSession>();
    let (vision, detection) = match senses {
        Senses::Ordinary => (AiVision::Filtered, AiDetection::Detectors),
        Senses::Given(vision, detection) => (vision, detection),
        Senses::SeatDeclared => match session.slot(player).and_then(PlayerSlot::senses) {
            Some(pair) => pair,
            None => return Sighting::Unseen,
        },
    };

    let entity_ref = world.entity(entity);
    if !entity_ref.contains::<LocationComponent>() {
        return Sighting::Unseen;
    }
    // Off the map — aboard, inside a site or a mine — nobody sees it, its own
    // side included: the owner knows it from what holds it, not by sight.
    if entity_ref.contains::<HiddenComponent>() {
        return Sighting::Unseen;
    }
    if owner::admits(
        session,
        Affiliation::Allied,
        Some(player),
        entity_def::owner(world, entity),
    ) {
        return Sighting::Seen;
    }
    // Every cell the entity stands on answers, not one of them: a hall is lit
    // when any of its cells is, and found when a detector reaches any of
    // them. Sight is stamped from the same footprint.
    let footprint = entity_def::occupied_rect_of(world, def, entity);
    let lit = match vision {
        AiVision::Omniscient => true,
        AiVision::Filtered => {
            let grid = world.resource::<VisibilityGrid>();
            footprint
                .cells()
                .any(|cell| grid.is_visible_to(session, player, cell.x, cell.y))
        }
    };
    if !lit {
        return Sighting::Unseen;
    }
    if !entity_ref.contains::<ConcealedComponent>() {
        return Sighting::Seen;
    }
    let detected = match detection {
        AiDetection::Everywhere => true,
        AiDetection::Detectors => {
            let standing_on = def
                .location
                .expect("validated content stands somewhere")
                .occupation();
            footprint.cells().any(|cell| {
                fields::detects(world, player, cell, standing_on)
                    || watches::detects(world, player, cell, standing_on)
            })
        }
    };
    if detected {
        Sighting::Seen
    } else {
        Sighting::Glimpsed
    }
}
