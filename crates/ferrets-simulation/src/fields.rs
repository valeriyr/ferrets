//! Fields: per-cell areas projected by standing entities, and the predicates
//! that read them.
//!
//! The grid is simulation state: coverage accumulates as sources grow and
//! lingers as it decays. The predicates store nothing and re-derive from the
//! grid every call.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};

use ferrets_physics::body;

use crate::{
    entity_def,
    session::{GameSession, player_id::PlayerId, player_mask::PlayerMask},
};
use ferrets_content::{
    entity_type_def::EntityTypeDef,
    field::{
        FieldAffiliation, FieldCoverage, FieldEffect, FieldEffectKind, FieldId, FieldPlacement,
        FieldSide,
    },
    stats::EntityModifier,
};

/// One field's cells.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldLayer {
    /// Who covers each cell, row-major.
    covered: Vec<PlayerMask>,
    /// Who sustains each cell this tick, row-major.
    sustained: Vec<PlayerMask>,
    /// Ticks until the next recession step of a gradually decaying field.
    decay_countdown: u32,
}

/// Every field's coverage over the map, indexed by [`FieldId`] then cell.
///
/// Layers come into being the first time a field is written, so the grid
/// follows whatever fields the content registers; a field never written reads
/// as clear everywhere.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct FieldGrid {
    width: u32,
    height: u32,
    fields: Vec<FieldLayer>,
}

impl FieldGrid {
    /// Creates an all-clear grid over a `width × height` map.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            fields: Vec::new(),
        }
    }

    /// The layer of `field`, if it has been written.
    fn layer(&self, field: FieldId) -> Option<&FieldLayer> {
        self.fields.get(field.index())
    }

    /// The layer of `field`, brought into being if it has not been.
    fn layer_mut(&mut self, field: FieldId) -> &mut FieldLayer {
        let len = (self.width * self.height) as usize;
        while self.fields.len() <= field.index() {
            self.fields.push(FieldLayer {
                covered: vec![PlayerMask::EMPTY; len],
                sustained: vec![PlayerMask::EMPTY; len],
                decay_countdown: 0,
            });
        }
        &mut self.fields[field.index()]
    }

    /// The grid's width in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The grid's height in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether `pos` lies on the grid.
    #[inline]
    pub fn contains(&self, pos: CellPos) -> bool {
        pos.x < self.width && pos.y < self.height
    }

    /// Who covers `pos` in `field`.
    pub fn covered(&self, field: FieldId, pos: CellPos) -> PlayerMask {
        let index = self.index(pos);
        self.layer(field)
            .map_or(PlayerMask::EMPTY, |layer| layer.covered[index])
    }

    /// Whether `pos` in `field` is covered by someone satisfying `of` judged
    /// from `player`.
    pub fn covers(
        &self,
        session: &GameSession,
        field: FieldId,
        pos: CellPos,
        of: FieldAffiliation,
        player: Option<PlayerId>,
    ) -> bool {
        self.covered(field, pos).satisfies(session, of, player)
    }

    /// Every cell of `field` with its coverage, row-major. Empty for a field
    /// never written.
    pub fn cells(&self, field: FieldId) -> impl Iterator<Item = (CellPos, PlayerMask)> + '_ {
        let width = self.width;
        self.layer(field)
            .map(|layer| layer.covered.as_slice())
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(move |(i, &mask)| (CellPos::new(i as u32 % width, i as u32 / width), mask))
    }

    /// Adds `player` to the cover of `pos` in `field`.
    pub(crate) fn cover(&mut self, field: FieldId, pos: CellPos, player: PlayerId) {
        let index = self.index(pos);
        self.layer_mut(field).covered[index] |= PlayerMask::of(player);
    }

    /// Removes every player not sustaining `pos` in `field` from its cover.
    pub(crate) fn clear_unsustained(&mut self, field: FieldId, pos: CellPos) {
        let index = self.index(pos);
        let layer = self.layer_mut(field);
        layer.covered[index] &= layer.sustained[index];
    }

    /// Forgets last tick's sustain of `field`.
    pub(crate) fn reset_sustained(&mut self, field: FieldId) {
        self.layer_mut(field).sustained.fill(PlayerMask::EMPTY);
    }

    /// Records that `player` sustains `pos` in `field` this tick.
    pub(crate) fn sustain(&mut self, field: FieldId, pos: CellPos, player: PlayerId) {
        let index = self.index(pos);
        self.layer_mut(field).sustained[index] |= PlayerMask::of(player);
    }

    /// Adds every sustaining player to the cover of every cell of `field`.
    pub(crate) fn grow(&mut self, field: FieldId) {
        let layer = self.layer_mut(field);
        for (covered, &sustained) in layer.covered.iter_mut().zip(&layer.sustained) {
            *covered |= sustained;
        }
    }

    /// Removes every unsustained player from the cover of every cell of
    /// `field`.
    pub(crate) fn clear_all_unsustained(&mut self, field: FieldId) {
        let layer = self.layer_mut(field);
        for (covered, &sustained) in layer.covered.iter_mut().zip(&layer.sustained) {
            *covered &= sustained;
        }
    }

    /// Removes every unsustained player from the cover of the cells of `field`
    /// that lie on the edge of that player's patch: a covered cell with a
    /// four-neighbour the player does not cover, or on the map's border.
    pub(crate) fn recede(&mut self, field: FieldId) {
        let (width, height) = (self.width as usize, self.height as usize);
        let layer = self.layer_mut(field);
        let snapshot = layer.covered.clone();
        for (index, covered) in layer.covered.iter_mut().enumerate() {
            let unsustained = snapshot[index] & !layer.sustained[index];
            if unsustained.is_empty() {
                continue;
            }
            let (x, y) = (index % width, index / width);
            let neighbours = [
                (x > 0).then(|| index - 1),
                (x + 1 < width).then(|| index + 1),
                (y > 0).then(|| index - width),
                (y + 1 < height).then(|| index + width),
            ];
            let mut interior = unsustained;
            for neighbour in neighbours {
                interior &= match neighbour {
                    Some(neighbour) => snapshot[neighbour],
                    None => PlayerMask::EMPTY,
                };
            }
            *covered &= !(unsustained & !interior);
        }
    }

    /// Ticks the recession countdown of `field`: returns `true` when a step
    /// is due and rearms the countdown to `cycle`.
    pub(crate) fn decay_due(&mut self, field: FieldId, cycle: u32) -> bool {
        let layer = self.layer_mut(field);
        if layer.decay_countdown == 0 {
            layer.decay_countdown = cycle;
        }
        layer.decay_countdown -= 1;
        layer.decay_countdown == 0
    }

    fn index(&self, pos: CellPos) -> usize {
        assert!(
            self.contains(pos),
            "cell ({}, {}) out of range ({}x{})",
            pos.x,
            pos.y,
            self.width,
            self.height
        );
        (pos.y * self.width + pos.x) as usize
    }
}

/// The field's reading of a cover mask: whom its coverage counts for.
impl PlayerMask {
    /// Whether any player in the mask satisfies `of` judged from `player`.
    pub fn satisfies(
        self,
        session: &GameSession,
        of: FieldAffiliation,
        player: Option<PlayerId>,
    ) -> bool {
        match of {
            FieldAffiliation::Anyone => !self.is_empty(),
            FieldAffiliation::Own => player.is_some_and(|player| self.contains(player)),
            FieldAffiliation::Allied => player.is_some_and(|player| {
                self.players()
                    .any(|other| session.are_allied(player, other))
            }),
        }
    }
}

/// The cells of `rect` a placement rule with `coverage` reads.
fn read_cells(rect: CellRect, coverage: FieldCoverage) -> Vec<CellPos> {
    match coverage {
        FieldCoverage::Anchor => vec![rect.origin],
        FieldCoverage::Footprint => (0..rect.size.height)
            .flat_map(|dy| {
                (0..rect.size.width)
                    .map(move |dx| CellPos::new(rect.origin.x + dx, rect.origin.y + dy))
            })
            .collect(),
    }
}

/// Whether the fields admit a placement of `def` anchored at `anchor` for
/// `player`. A def with no placement rules is always admitted.
pub fn allows_placement(
    world: &World,
    player: Option<PlayerId>,
    def: &EntityTypeDef,
    anchor: CellPos,
) -> bool {
    allows_placement_in(
        world.resource::<FieldGrid>(),
        world.resource::<GameSession>(),
        player,
        def,
        anchor,
    )
}

/// [`allows_placement`] against the given grid and session.
pub fn allows_placement_in(
    grid: &FieldGrid,
    session: &GameSession,
    player: Option<PlayerId>,
    def: &EntityTypeDef,
    anchor: CellPos,
) -> bool {
    if def.field_placement.is_empty() {
        return true;
    }
    let size = def
        .location
        .expect("validated content defines a location")
        .size();
    let rect = CellRect::new(anchor, size);
    def.field_placement.iter().all(|rule| match *rule {
        FieldPlacement::Requires {
            field,
            of,
            coverage,
        } => read_cells(rect, coverage)
            .into_iter()
            .all(|cell| grid.contains(cell) && grid.covers(session, field, cell, of, player)),
        FieldPlacement::Forbids { field } => read_cells(rect, FieldCoverage::Footprint)
            .into_iter()
            .all(|cell| !grid.contains(cell) || grid.covered(field, cell).is_empty()),
    })
}

/// Whether the effect applies to an entity owned by `player` whose anchor is
/// `anchor`.
fn effect_applies(
    grid: &FieldGrid,
    session: &GameSession,
    effect: &FieldEffect,
    player: Option<PlayerId>,
    anchor: CellPos,
) -> bool {
    let covered =
        grid.contains(anchor) && grid.covers(session, effect.field(), anchor, effect.of(), player);
    match effect.side() {
        FieldSide::Inside => covered,
        FieldSide::Outside => !covered,
    }
}

/// The anchor cell and owner a field effect on `entity` is judged by.
fn standing(world: &World, entity: Entity) -> (Option<PlayerId>, CellPos) {
    (
        entity_def::owner(world, entity),
        body::anchor(entity_def::position(world, entity)),
    )
}

/// Whether `entity` stands disabled: some field effect of its type says so
/// for the side of the field its anchor cell is on.
pub fn disabled(world: &World, entity: Entity) -> bool {
    let def = entity_def::of(world, entity);
    if def.field_effects.is_empty() {
        return false;
    }
    let (player, anchor) = standing(world, entity);
    disabled_in(
        world.resource::<FieldGrid>(),
        world.resource::<GameSession>(),
        def,
        player,
        anchor,
    )
}

/// Whether an entity of `def` owned by `player` with its anchor at `anchor`
/// stands disabled, against the given grid and session.
pub fn disabled_in(
    grid: &FieldGrid,
    session: &GameSession,
    def: &EntityTypeDef,
    player: Option<PlayerId>,
    anchor: CellPos,
) -> bool {
    def.field_effects.iter().any(|effect| match effect.kind() {
        FieldEffectKind::Disabled => effect_applies(grid, session, effect, player, anchor),
        FieldEffectKind::Modifiers(_) => false,
    })
}

/// The modifiers the fields currently fold into `entity`'s stats.
pub fn modifiers(world: &World, entity: Entity) -> Vec<EntityModifier> {
    let def = entity_def::of(world, entity);
    if def.field_effects.is_empty() {
        return Vec::new();
    }
    let grid = world.resource::<FieldGrid>();
    let session = world.resource::<GameSession>();
    let (player, anchor) = standing(world, entity);
    def.field_effects
        .iter()
        .filter(|effect| effect_applies(grid, session, effect, player, anchor))
        .flat_map(|effect| match effect.kind() {
            FieldEffectKind::Modifiers(modifiers) => modifiers.as_slice(),
            FieldEffectKind::Disabled => &[],
        })
        .copied()
        .collect()
}
