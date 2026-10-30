//! Shadows on the fog of war: the cells that block sight in a tick, and
//! whether a viewer's line of sight to a cell passes one.

use std::collections::BTreeMap;

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{affiliation::Affiliation, registry::ContentRegistry};
use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    grid_line::{self, Crossing},
};
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{
    components::{location::LocationComponent, owner},
    entity_def,
    entity_index::EntityIndex,
    fields::FieldGrid,
    map::Map,
    session::{GameSession, player_id::PlayerId, player_mask::PlayerMask},
};

/// Every cell that blocks sight in a tick.
pub(crate) struct Blockers {
    /// The obstructions on each blocking cell.
    cells: BTreeMap<CellPos, Vec<Obstruction>>,
}

impl Blockers {
    /// Gathers the cells of every object on the map, alive or dying, whose
    /// type blocks sight, its top its level there plus its height, and every
    /// cell a sight-blocking field covers, its top the cell's highest level on
    /// the layers it spreads on plus the field's height — the top it reaches
    /// on each of those layers, the highest standing above the others.
    pub(crate) fn gather(world: &World) -> Self {
        let registry = world.resource::<ContentRegistry>();
        let grid = world.resource::<Map>().nav_grid();
        let index = world.resource::<EntityIndex>();
        let mut cells: BTreeMap<CellPos, Vec<Obstruction>> = BTreeMap::new();

        for (_, entity) in index
            .alive_entries()
            .into_iter()
            .chain(index.dying_entries())
        {
            let def = entity_def::of(world, entity);
            let Some(block) = def.blocks_sight else {
                continue;
            };
            if !world.entity(entity).contains::<LocationComponent>() {
                continue;
            }
            let obstruction = Obstruction {
                blocker: Blocker::Object {
                    entity,
                    owner: entity_def::owner(world, entity),
                },
                top: entity_def::level(world, entity).saturating_add(block.height()),
                of: block.of(),
            };
            for cell in entity_def::occupied_rect_of(world, def, entity).cells() {
                cells.entry(cell).or_default().push(obstruction);
            }
        }

        if let Some(fields) = world.get_resource::<FieldGrid>() {
            for field in registry.field_ids() {
                let def = registry.field_def(field);
                let Some(block) = def.blocks_sight() else {
                    continue;
                };
                let layers: Vec<_> = registry
                    .layers()
                    .map(|(_, layer)| layer)
                    .filter(|&layer| def.spreads_on() & layer != LayerMask::EMPTY)
                    .collect();
                for (cell, coverers) in fields.covered_cells(field) {
                    let level = layers
                        .iter()
                        .map(|&layer| {
                            grid.cell_level(layer, cell)
                                .expect("a covered cell lies on the map")
                        })
                        .max()
                        .expect("a registered field spreads on a layer");
                    cells.entry(cell).or_default().push(Obstruction {
                        blocker: Blocker::Field { coverers },
                        top: level.saturating_add(block.height()),
                        of: block.of(),
                    });
                }
            }
        }

        Self { cells }
    }

    /// The shade the blockers cast for a viewer of `player` whose eyes are at
    /// `eyes`, standing on `standing` and seeing `sight` cells around it: the
    /// blocking cells within its sight's bounding square, its own cells left
    /// out, whose top is above its eyes and whose `of` names its player.
    pub(crate) fn shade(
        &self,
        session: &GameSession,
        player: PlayerId,
        eyes: u32,
        standing: CellRect,
        sight: u32,
    ) -> Shade {
        let bounds = sight_bounds(standing, sight);
        let (first, last) = (
            bounds.origin,
            CellPos::new(
                bounds.origin.x + bounds.size.width - 1,
                bounds.origin.y + bounds.size.height - 1,
            ),
        );
        // Cells order by column, then row: the range holds every column of the
        // bounds, the filter keeps the rows.
        let cells = self
            .cells
            .range(first..=last)
            .filter(|&(&cell, _)| bounds.contains(cell) && !standing.contains(cell))
            .filter_map(|(&cell, obstructions)| {
                let blocking: Vec<Blocker> = obstructions
                    .iter()
                    .filter(|obstruction| {
                        obstruction.top > eyes && blocks(session, obstruction, player)
                    })
                    .map(|obstruction| obstruction.blocker)
                    .collect();
                (!blocking.is_empty()).then_some((cell, blocking))
            })
            .collect();
        Shade { standing, cells }
    }
}

/// The cells blocking one viewer's sight.
pub(crate) struct Shade {
    /// The cells the viewer stands on, seeing from their center.
    standing: CellRect,
    /// The blockers on each cell that blocks it.
    cells: BTreeMap<CellPos, Vec<Blocker>>,
}

impl Shade {
    /// Whether nothing blocks the viewer's sight.
    pub(crate) fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether `target` lies in the shadow: the segment from the center of
    /// the viewer's cells to its center passes through a blocking cell other
    /// than itself, or through the corner two blocking cells diagonal to each
    /// other share. An object's cells never hide one another.
    pub(crate) fn hides(&self, target: CellPos) -> bool {
        // Every cell and corner the segment passes lies within the rectangle
        // its two ends span: with no blocking cell there, nothing is walked.
        let between = self.standing.union(CellRect::cell(target));
        if !self
            .cells
            .keys()
            .any(|&cell| cell != target && between.contains(cell))
        {
            return false;
        }
        let closes = |cell: CellPos| {
            cell != target
                && self.cells.get(&cell).is_some_and(|blockers| {
                    blockers.iter().any(|blocker| match blocker {
                        Blocker::Object { entity, .. } => !self.stands_on(*entity, target),
                        Blocker::Field { .. } => true,
                    })
                })
        };
        grid_line::crossings(self.standing, CellRect::cell(target)).any(|crossing| match crossing {
            Crossing::Cell(cell) => closes(cell),
            Crossing::Corner(corner) => match (corner.x.checked_sub(1), corner.y.checked_sub(1)) {
                (Some(west), Some(north)) => {
                    (closes(CellPos::new(west, north)) && closes(corner))
                        || (closes(CellPos::new(corner.x, north))
                            && closes(CellPos::new(west, corner.y)))
                }
                // A corner on the map's west or north edge has no cells
                // beyond it to close a line.
                (None, _) | (_, None) => false,
            },
        })
    }

    /// Whether the object `entity` stands on `cell`.
    fn stands_on(&self, entity: Entity, cell: CellPos) -> bool {
        self.cells.get(&cell).is_some_and(|blockers| {
            blockers.iter().any(|blocker| match blocker {
                Blocker::Object { entity: other, .. } => *other == entity,
                Blocker::Field { .. } => false,
            })
        })
    }
}

/// What blocks sight on a cell.
#[derive(Debug, Clone, Copy)]
enum Blocker {
    /// A cell of an object's footprint.
    Object {
        /// The object.
        entity: Entity,
        /// The object's owner, if any.
        owner: Option<PlayerId>,
    },
    /// A cell a field covers.
    Field {
        /// The players covering the cell.
        coverers: PlayerMask,
    },
}

/// One blocker on a cell, with how high it rises and whom it blocks.
#[derive(Debug, Clone, Copy)]
struct Obstruction {
    /// What blocks.
    blocker: Blocker,
    /// The level its top is at: a viewer whose eyes are below it is blocked.
    top: u32,
    /// Whose sight it blocks, judged from its owner or its coverers.
    of: Affiliation,
}

/// Whether `obstruction` blocks the sight of `player`, judged from its owner
/// or its coverers.
fn blocks(session: &GameSession, obstruction: &Obstruction, player: PlayerId) -> bool {
    match obstruction.blocker {
        Blocker::Object { owner, .. } => {
            owner::admits(session, obstruction.of, Some(player), owner)
        }
        Blocker::Field { coverers } => coverers.satisfies(session, obstruction.of, Some(player)),
    }
}

/// The bounding square of the cells within `sight` of `standing`.
fn sight_bounds(standing: CellRect, sight: u32) -> CellRect {
    let origin = CellPos::new(
        standing.origin.x.saturating_sub(sight),
        standing.origin.y.saturating_sub(sight),
    );
    CellRect::new(
        origin,
        CellSize::new(
            (standing.origin.x + standing.size.width).saturating_add(sight) - origin.x,
            (standing.origin.y + standing.size.height).saturating_add(sight) - origin.y,
        ),
    )
}
