//! Per-tick fog of war recompute: re-stamps each player's visible cells from
//! the sight of their owned entities, the fields whose watch names them, and
//! the watches they hold over patches of the map; relief and shadows limit a
//! body's own sight.

use bevy_ecs::{prelude::*, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, projection};
use ferrets_nav_grid::{layer_id::LayerId, layer_mask::LayerMask};

use crate::{
    components::{
        entity_stats::StatsComponent, location::LocationComponent, owner::OwnerComponent,
    },
    entity_def,
    fields::FieldGrid,
    map::Map,
    session::{GameSession, player_id::PlayerId, player_mask::PlayerMask, player_slot::PlayerSlot},
    shadows::Blockers,
    visibility::VisibilityGrid,
    watches::{Watch, Watches},
};
use ferrets_content::{entity_stats::EntityStatId, registry::ContentRegistry};

/// Recomputes the visibility grid for the current tick.
///
/// Last tick's visible cells demote to explored (sticky), then every owned,
/// on-map entity reveals for its player every cell within its `sight_range`
/// of the cells it occupies that neither relief nor a shadow hides from its
/// eyes, every cell a watching field covers is revealed to each player its
/// watch names, judged from the cell's coverers, and every watch still in
/// force this tick reveals the patch it holds to the player who cast it. A
/// body's eyes are its level, the highest of its layer under the cells it
/// occupies, plus its `sees_up`; a cell is hidden from it when, on any layer
/// whose relief blocks the sight of the body's layer, the cell's level is
/// above its eyes, or when the line from the center of its cells to the
/// cell's center passes a cell an object or a field blocks it on, the
/// blocker's top above its eyes and its `of` naming the body's player.
/// Fields and watches reveal their cells whatever blocks them. The result is
/// a pure function of entity footprints, the sight stats, the map's levels,
/// the blockers standing, this tick's field coverage, the watches in force,
/// and team membership, so it is identical on every node.
pub fn recompute_visibility(world: &mut World) {
    // Owned, on-map seers: (entity, player, sight radius, `sees_up`), read
    // from the effective stat store; an unset sight sees only the cells
    // occupied, an unset `sees_up` nothing above its level. A raw query
    // rather than the alive index, so something still dying goes on seeing —
    // it is not dead yet — while a body sees nothing at all, carrying neither
    // an owner nor a stat store. The OR-fold below is commutative, so
    // iteration order cannot reach the shared grid.
    let seers: Vec<(Entity, PlayerId, u32, u32)> = world
        .query_filtered::<(Entity, &OwnerComponent, &StatsComponent), With<LocationComponent>>()
        .iter(world)
        .map(|(entity, owner, stats)| {
            (
                entity,
                owner.player(),
                stats
                    .effective_as_u32(EntityStatId::SIGHT_RANGE)
                    .unwrap_or(0),
                stats.effective_as_u32(EntityStatId::SEES_UP).unwrap_or(0),
            )
        })
        .collect();
    // Each seer as a source: (player, occupied cells, sight radius, eyes, the
    // layers whose relief blocks its sight). No two cells lie further apart
    // than the map's width and height together, so no sight reaches further.
    let map_span = {
        let map = world.resource::<Map>();
        map.width().saturating_add(map.height())
    };
    let sources: Vec<(PlayerId, CellRect, u32, u32, Vec<LayerId>)> = seers
        .into_iter()
        .map(|(entity, player, sight, sees_up)| {
            let sight = sight.min(map_span);
            let standing = entity_def::occupied_rect(world, entity);
            let registry = world.resource::<ContentRegistry>();
            let layer = entity_def::layer(world, entity);
            let level = entity_def::level(world, entity);
            let blocked_by = registry
                .layers()
                .filter(|&(_, relief)| {
                    registry.layer_def(relief).relief_blocks() & layer != LayerMask::EMPTY
                })
                .map(|(_, relief)| relief)
                .collect();
            (
                player,
                standing,
                sight,
                level.saturating_add(sees_up),
                blocked_by,
            )
        })
        .collect();

    // Cells a watching field covers, for every player its watch names judged
    // from the cell's coverers. Coverage is a per-cell owner set, so the fold
    // below is commutative too. A world with no field grid has no fields to
    // watch through.
    let field_cells: Vec<(PlayerId, CellPos)> = match (
        world.get_resource::<FieldGrid>(),
        world.get_resource::<ContentRegistry>(),
    ) {
        (Some(fields), Some(registry)) => {
            let session = world.resource::<GameSession>();
            let mut cells = Vec::new();
            for field in registry.field_ids() {
                let Some(watch) = registry.field_def(field).watches() else {
                    continue;
                };
                // The players each set of coverers reveals its cells to,
                // judged once per set: a patch's cells mostly share one.
                let mut watchers: Vec<(PlayerMask, PlayerMask)> = Vec::new();
                for (cell, covering) in fields.covered_cells(field) {
                    let revealed_to = match watchers.iter().find(|(set, _)| *set == covering) {
                        Some(&(_, revealed_to)) => revealed_to,
                        None => {
                            let revealed_to = session
                                .occupied_slots()
                                .map(PlayerSlot::id)
                                .filter(|&player| {
                                    covering.satisfies(session, watch.of(), Some(player))
                                })
                                .fold(PlayerMask::EMPTY, |mask, player| {
                                    mask | PlayerMask::of(player)
                                });
                            watchers.push((covering, revealed_to));
                            revealed_to
                        }
                    };
                    cells.extend(revealed_to.players().map(|player| (player, cell)));
                }
            }
            cells
        }
        _ => Vec::new(),
    };

    // The cells that block sight this tick, judged per viewer below. Order
    // cannot reach the grid: a cell is hidden from a viewer or not, whatever
    // order the blockers were gathered in.
    let blockers = Blockers::gather(world);
    let revealed: Vec<(PlayerId, CellPos)> = {
        let map = world.resource::<Map>();
        let grid = map.nav_grid();
        let session = world.resource::<GameSession>();
        sources
            .into_iter()
            .flat_map(|(player, standing, sight, eyes, blocked_by)| {
                let shade = blockers.shade(session, player, eyes, standing, sight);
                projection::circle_cells(standing, sight)
                    .into_iter()
                    .filter(|&cell| map.contains(cell))
                    .filter(move |&cell| {
                        blocked_by.iter().all(|&relief| {
                            grid.cell_level(relief, cell)
                                .expect("a cell on the map has a level on every layer")
                                <= eyes
                        })
                    })
                    .filter(move |&cell| shade.is_empty() || !shade.hides(cell))
                    .map(move |cell| (player, cell))
            })
            .collect()
    };

    // The patches watches hold. Everything the store holds is in force: a
    // watch is dropped by the pass that takes its last tick off it.
    let watches: Vec<Watch> = world.resource::<Watches>().in_force().to_vec();
    let watch_cells: Vec<(PlayerId, CellPos)> = {
        let map = world.resource::<Map>();
        watches
            .iter()
            .flat_map(|watch| {
                watch
                    .cells()
                    .into_iter()
                    .filter(|&cell| map.contains(cell))
                    .map(move |cell| (watch.player, cell))
            })
            .collect()
    };

    let mut grid = world.resource_mut::<VisibilityGrid>();
    grid.age();
    for (player, cell) in revealed {
        grid.reveal(player, cell.x, cell.y);
    }
    for (player, cell) in field_cells {
        grid.reveal(player, cell.x, cell.y);
    }
    for (player, cell) in watch_cells {
        grid.reveal(player, cell.x, cell.y);
    }
}
