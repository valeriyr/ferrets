//! Per-tick fog of war recompute: re-stamps each player's visible cells from
//! the sight of their owned entities and the fields they cover.

use bevy_ecs::{prelude::*, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, projection};

use crate::{
    components::{
        entity_stats::StatsComponent, hidden::HiddenComponent, location::LocationComponent,
        owner::OwnerComponent,
    },
    entity_def,
    fields::FieldGrid,
    map::Map,
    session::player_id::PlayerId,
    visibility::VisibilityGrid,
};
use ferrets_content::{entity_stats::EntityStatId, field::FieldVision, registry::ContentRegistry};

/// Recomputes the visibility grid for the current tick.
///
/// Last tick's visible cells demote to explored (sticky), then every owned,
/// on-map entity reveals every cell within its `sight_range` of the cells it
/// occupies for its player, and every cell of a field that grants vision is
/// revealed to each player covering it. The result is a pure function of
/// entity footprints, the (static) sight stat, this tick's field coverage, and
/// team membership, so it is identical on every node.
pub fn recompute_visibility(world: &mut World) {
    // Owned, on-map sight sources: (player, occupied cells, sight radius). Sight
    // is read from the effective stat store; an unset sight sees only the cells
    // occupied. A raw query rather than the alive index: the dying still see
    // until their remains leave the map. The OR-fold below is commutative, so
    // iteration order cannot reach the shared grid.
    let seers: Vec<(Entity, PlayerId, u32)> = world
        .query_filtered::<(Entity, &OwnerComponent, &StatsComponent), (With<LocationComponent>, Without<HiddenComponent>)>()
        .iter(world)
        .map(|(entity, owner, stats)| {
            (
                entity,
                owner.player(),
                stats.effective_as_u32(EntityStatId::SIGHT_RANGE).unwrap_or(0),
            )
        })
        .collect();
    let sources: Vec<(PlayerId, CellRect, u32)> = seers
        .into_iter()
        .map(|(entity, player, sight)| (player, entity_def::occupied_rect(world, entity), sight))
        .collect();

    // Cells a watching field covers. Coverage is a per-cell owner set, so the
    // fold below is commutative too. A world with no field grid has no fields
    // to watch through.
    let watched: Vec<(PlayerId, CellPos)> = match (
        world.get_resource::<FieldGrid>(),
        world.get_resource::<ContentRegistry>(),
    ) {
        (Some(fields), Some(registry)) => registry
            .field_ids()
            .filter(|&field| match registry.field_def(field).vision() {
                FieldVision::Watched => true,
                FieldVision::Dark => false,
            })
            .flat_map(|field| {
                fields.cells(field).flat_map(|(cell, covering)| {
                    covering.players().map(move |player| (player, cell))
                })
            })
            .collect(),
        _ => Vec::new(),
    };

    let revealed: Vec<(PlayerId, CellPos)> = {
        let map = world.resource::<Map>();
        sources
            .into_iter()
            .flat_map(|(player, standing, sight)| {
                projection::circle_cells(standing, sight)
                    .into_iter()
                    .filter(|&cell| map.contains(cell))
                    .map(move |cell| (player, cell))
            })
            .collect()
    };

    let mut grid = world.resource_mut::<VisibilityGrid>();
    grid.age();
    for (player, cell) in revealed {
        grid.reveal(player, cell.x, cell.y);
    }
    for (player, cell) in watched {
        grid.reveal(player, cell.x, cell.y);
    }
}
