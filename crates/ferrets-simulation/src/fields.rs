//! Fields: per-cell areas projected by standing entities, and the predicates
//! that read them.
//!
//! The grid is simulation state: coverage accumulates as sources grow and
//! lingers as it decays. The predicates store nothing and re-derive from the
//! grid every call.

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};

use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{
    components::owner,
    entity_def, requirements,
    session::{GameSession, player_id::PlayerId, player_mask::PlayerMask},
};
use ferrets_content::{
    affiliation::Affiliation,
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_type_def::EntityTypeDef,
    field::{FieldCoverage, FieldEffect, FieldId, FieldPlacement, FieldSide},
    registry::ContentRegistry,
    requirement::Requirement,
};

/// One field's cells.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldCells {
    /// Who covers each cell, row-major.
    covered: Vec<PlayerMask>,
    /// Who sustains each cell this tick, row-major.
    sustained: Vec<PlayerMask>,
    /// Ticks until the next recession step of a gradually decaying field.
    decay_countdown: u32,
}

/// Every field's coverage over the map, indexed by [`FieldId`] then cell.
///
/// A field's cells come into being the first time it is written, so the grid
/// follows whatever fields the content registers; a field never written reads
/// as clear everywhere.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct FieldGrid {
    width: u32,
    height: u32,
    fields: Vec<FieldCells>,
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

    /// The cells of `field`, if it has been written.
    fn cells_of(&self, field: FieldId) -> Option<&FieldCells> {
        self.fields.get(field.index())
    }

    /// The cells of `field`, brought into being if they have not been.
    fn cells_of_mut(&mut self, field: FieldId) -> &mut FieldCells {
        let len = (self.width * self.height) as usize;
        while self.fields.len() <= field.index() {
            self.fields.push(FieldCells {
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
        self.cells_of(field)
            .map_or(PlayerMask::EMPTY, |cells| cells.covered[index])
    }

    /// Whether `pos` in `field` is covered by someone satisfying `of` judged
    /// from `player`.
    pub fn covers(
        &self,
        session: &GameSession,
        field: FieldId,
        pos: CellPos,
        of: Affiliation,
        player: Option<PlayerId>,
    ) -> bool {
        self.covered(field, pos).satisfies(session, of, player)
    }

    /// Every cell of `field` with its coverage, row-major. Empty for a field
    /// never written.
    pub fn cells(&self, field: FieldId) -> impl Iterator<Item = (CellPos, PlayerMask)> + '_ {
        let width = self.width;
        self.cells_of(field)
            .map(|cells| cells.covered.as_slice())
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(move |(i, &mask)| (CellPos::new(i as u32 % width, i as u32 / width), mask))
    }

    /// Every cell of `field` some player covers, with its coverage,
    /// row-major.
    pub fn covered_cells(
        &self,
        field: FieldId,
    ) -> impl Iterator<Item = (CellPos, PlayerMask)> + '_ {
        self.cells(field)
            .filter(|(_, covering)| !covering.is_empty())
    }

    /// Adds `player` to the cover of `pos` in `field`.
    pub(crate) fn cover(&mut self, field: FieldId, pos: CellPos, player: PlayerId) {
        let index = self.index(pos);
        self.cells_of_mut(field).covered[index] |= PlayerMask::of(player);
    }

    /// Removes every player not sustaining `pos` in `field` from its cover.
    pub(crate) fn clear_unsustained(&mut self, field: FieldId, pos: CellPos) {
        let index = self.index(pos);
        let cells = self.cells_of_mut(field);
        cells.covered[index] &= cells.sustained[index];
    }

    /// Forgets last tick's sustain of `field`.
    pub(crate) fn reset_sustained(&mut self, field: FieldId) {
        self.cells_of_mut(field).sustained.fill(PlayerMask::EMPTY);
    }

    /// Records that `player` sustains `pos` in `field` this tick.
    pub(crate) fn sustain(&mut self, field: FieldId, pos: CellPos, player: PlayerId) {
        let index = self.index(pos);
        self.cells_of_mut(field).sustained[index] |= PlayerMask::of(player);
    }

    /// Adds every sustaining player to the cover of every cell of `field`.
    pub(crate) fn grow(&mut self, field: FieldId) {
        let cells = self.cells_of_mut(field);
        for (covered, &sustained) in cells.covered.iter_mut().zip(&cells.sustained) {
            *covered |= sustained;
        }
    }

    /// Removes every unsustained player from the cover of every cell of
    /// `field`.
    pub(crate) fn clear_all_unsustained(&mut self, field: FieldId) {
        let cells = self.cells_of_mut(field);
        for (covered, &sustained) in cells.covered.iter_mut().zip(&cells.sustained) {
            *covered &= sustained;
        }
    }

    /// Removes every unsustained player from the cover of the cells of `field`
    /// that lie on the edge of that player's patch: a covered cell with a
    /// four-neighbour the player does not cover, or on the map's border.
    pub(crate) fn recede(&mut self, field: FieldId) {
        let (width, height) = (self.width as usize, self.height as usize);
        let cells = self.cells_of_mut(field);
        let snapshot = cells.covered.clone();
        for (index, covered) in cells.covered.iter_mut().enumerate() {
            let unsustained = snapshot[index] & !cells.sustained[index];
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
        let cells = self.cells_of_mut(field);
        if cells.decay_countdown == 0 {
            cells.decay_countdown = cycle;
        }
        cells.decay_countdown -= 1;
        cells.decay_countdown == 0
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
    /// Whether any player in the mask is whose `of` names, judged from
    /// `player`.
    ///
    /// Coverage belongs to whoever laid it, so an empty mask is nobody's and
    /// satisfies nothing — not even [`Affiliation::Anyone`], which names anyone
    /// who is there rather than the absence of cover.
    pub fn satisfies(
        self,
        session: &GameSession,
        of: Affiliation,
        player: Option<PlayerId>,
    ) -> bool {
        self.players()
            .any(|other| owner::admits(session, of, player, Some(other)))
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
    let size = def
        .location
        .expect("validated content defines a location")
        .size();
    debug_assert_spreads_on(
        world.resource::<ContentRegistry>(),
        def,
        def.field_placement.iter().map(FieldPlacement::field),
    );
    rules_allow(
        world,
        player,
        &def.field_placement,
        CellRect::new(anchor, size),
    )
}

/// Whether every one of `rules` admits a footprint at `rect` for `player`.
/// No rules admit anything.
pub fn rules_allow(
    world: &World,
    player: Option<PlayerId>,
    rules: &[FieldPlacement],
    rect: CellRect,
) -> bool {
    if rules.is_empty() {
        return true;
    }
    rules_allow_in(
        world.resource::<FieldGrid>(),
        world.resource::<GameSession>(),
        player,
        rules,
        rect,
    )
}

/// [`rules_allow`] against the given grid and session.
pub fn rules_allow_in(
    grid: &FieldGrid,
    session: &GameSession,
    player: Option<PlayerId>,
    rules: &[FieldPlacement],
    rect: CellRect,
) -> bool {
    rules_allow_by(session, player, rules, rect, |field, cell| {
        grid.contains(cell).then(|| grid.covered(field, cell))
    })
}

/// [`rules_allow`] over the coverage `covered` reads: who covers a field at a
/// cell, or `None` for a cell off the grid, which no field covers and every
/// requirement refuses.
pub fn rules_allow_by(
    session: &GameSession,
    player: Option<PlayerId>,
    rules: &[FieldPlacement],
    rect: CellRect,
    covered: impl Fn(FieldId, CellPos) -> Option<PlayerMask>,
) -> bool {
    rules.iter().all(|rule| match *rule {
        FieldPlacement::Requires {
            field,
            of,
            coverage,
        } => covers_enough(rect, coverage, |cell| {
            covered(field, cell).is_some_and(|mask| mask.satisfies(session, of, player))
        }),
        FieldPlacement::Forbids { field } => covers_enough(rect, FieldCoverage::Every, |cell| {
            covered(field, cell).is_none_or(PlayerMask::is_empty)
        }),
    })
}

/// Whether `entity` stands disabled: a field effect of its type disables it
/// on the side of the field it stands on. A hidden entity stands on no field.
pub fn disabled(world: &World, entity: Entity) -> bool {
    holds_effect(
        world,
        entity_def::of(world, entity),
        entity,
        |effect| match effect {
            EntityEffect::Disable => true,
            EntityEffect::Modifiers(_) | EntityEffect::Conceal => false,
        },
    )
}

/// Whether `entity` stands concealed by a field: a field effect of its type
/// conceals it on the side of the field it stands on. A hidden entity stands
/// on no field.
pub fn concealed(world: &World, entity: Entity) -> bool {
    concealed_of(world, entity_def::of(world, entity), entity)
}

/// [`concealed`] for an `entity` whose type `def` the caller already holds.
pub fn concealed_of(world: &World, def: &EntityTypeDef, entity: Entity) -> bool {
    holds_effect(world, def, entity, |effect| match effect {
        EntityEffect::Conceal => true,
        EntityEffect::Modifiers(_) | EntityEffect::Disable => false,
    })
}

/// Whether `player`'s side's detection over `cell` reaches something standing
/// on `layers`: some field whose detection names one of them covers the cell
/// for the player or an ally. A world with no field grid detects nothing.
pub fn detects(world: &World, player: PlayerId, cell: CellPos, layers: LayerMask) -> bool {
    let (Some(grid), Some(registry)) = (
        world.get_resource::<FieldGrid>(),
        world.get_resource::<ContentRegistry>(),
    ) else {
        return false;
    };
    if !grid.contains(cell) {
        return false;
    }
    let session = world.resource::<GameSession>();
    registry.detecting_fields().any(|(field, revealed)| {
        revealed & layers != LayerMask::EMPTY
            && grid.covers(session, field, cell, Affiliation::Allied, Some(player))
    })
}

/// The entity modifiers the fields currently fold into `entity`'s stats: the
/// sets of each field effect's side it stands on, while the effect holds;
/// none while it is hidden.
pub fn entity_modifiers(world: &World, entity: Entity) -> Vec<EntityModifiers> {
    entity_modifiers_of(world, entity, entity_def::of(world, entity))
}

/// The entity modifiers the field effects of the form `def` would fold into
/// `entity`'s stats, were it standing where it stands now as `def`: the sets
/// of each effect's side its footprint is on, while the effect holds; none
/// while it is hidden.
pub fn entity_modifiers_of(
    world: &World,
    entity: Entity,
    def: &EntityTypeDef,
) -> Vec<EntityModifiers> {
    if def.field_effects.is_empty() || entity_def::hidden(world, entity) {
        return Vec::new();
    }
    debug_assert_spreads_on(
        world.resource::<ContentRegistry>(),
        def,
        def.field_effects.iter().map(FieldEffect::field),
    );
    let grid = world.resource::<FieldGrid>();
    let session = world.resource::<GameSession>();
    let player = entity_def::owner(world, entity);
    let footprint = entity_def::occupied_rect_of(world, def, entity);
    let holds =
        |requirement: &Requirement| requirements::met_by(world, player, entity, requirement);
    def.field_effects
        .iter()
        .filter(|field_effect| {
            field_effect.effects().any(|effect| match effect {
                EntityEffect::Modifiers(_) => true,
                EntityEffect::Disable | EntityEffect::Conceal => false,
            })
        })
        .flat_map(|field_effect| applying(grid, session, field_effect, player, footprint, &holds))
        .filter_map(|effect| match effect {
            EntityEffect::Modifiers(modifiers) => Some(modifiers.clone()),
            EntityEffect::Disable | EntityEffect::Conceal => None,
        })
        .collect()
}

/// Panics in debug builds if one of `fields`, read for a body of `def`, does
/// not spread on the layer of its domain.
pub(crate) fn debug_assert_spreads_on(
    registry: &ContentRegistry,
    def: &EntityTypeDef,
    fields: impl IntoIterator<Item = FieldId>,
) {
    if let Some(location) = &def.location {
        let layer = registry.domain_def(location.domain()).layer();
        for field in fields {
            debug_assert!(
                registry.field_def(field).spreads_on() & layer != LayerMask::EMPTY,
                "the registry admits a field read for '{}' only where it spreads on its domain's layer",
                def.name
            );
        }
    }
}

/// Whether enough of `rect` answers `test` for `coverage` to be satisfied.
fn covers_enough(rect: CellRect, coverage: FieldCoverage, test: impl Fn(CellPos) -> bool) -> bool {
    match coverage {
        FieldCoverage::Every => rect.cells().all(test),
        FieldCoverage::Any => rect.cells().any(test),
    }
}

/// Whether a field effect of the form `def` does to `entity`, where it stands,
/// something `wanted` picks out. Entries carrying nothing wanted are passed
/// over unjudged.
fn holds_effect(
    world: &World,
    def: &EntityTypeDef,
    entity: Entity,
    wanted: impl Fn(&EntityEffect) -> bool,
) -> bool {
    if entity_def::hidden(world, entity) {
        return false;
    }
    let mut candidates = def
        .field_effects
        .iter()
        .filter(|field_effect| field_effect.effects().any(&wanted))
        .peekable();
    if candidates.peek().is_none() {
        return false;
    }
    debug_assert_spreads_on(
        world.resource::<ContentRegistry>(),
        def,
        def.field_effects.iter().map(FieldEffect::field),
    );
    let grid = world.resource::<FieldGrid>();
    let session = world.resource::<GameSession>();
    let player = entity_def::owner(world, entity);
    let footprint = entity_def::occupied_rect_of(world, def, entity);
    let holds =
        |requirement: &Requirement| requirements::met_by(world, player, entity, requirement);
    candidates.any(|field_effect| {
        applying(grid, session, field_effect, player, footprint, &holds)
            .iter()
            .any(&wanted)
    })
}

/// What `field_effect` does to an entity owned by `player` standing on
/// `footprint`: the effects of the side it stands on — inside when the
/// coverage is met, outside otherwise — or nothing when that side names none
/// or what the effect holds while is unmet.
fn applying<'a>(
    grid: &FieldGrid,
    session: &GameSession,
    field_effect: &'a FieldEffect,
    player: Option<PlayerId>,
    footprint: CellRect,
    holds: &impl Fn(&Requirement) -> bool,
) -> &'a [EntityEffect] {
    let inside = covers_enough(footprint, field_effect.coverage(), |cell| {
        grid.contains(cell)
            && grid.covers(
                session,
                field_effect.field(),
                cell,
                field_effect.of(),
                player,
            )
    });
    let side = match inside {
        true => FieldSide::Inside,
        false => FieldSide::Outside,
    };
    let effects = field_effect.on(side);
    if effects.is_empty() {
        return &[];
    }
    match field_effect.holds_while() {
        Some(requirement) if !holds(requirement) => &[],
        Some(_) | None => effects,
    }
}
