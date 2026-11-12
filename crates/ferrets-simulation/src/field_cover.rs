//! Writes to the field grid outside the per-tick recompute, and the flood a
//! field spreads by.

use std::collections::{HashSet, VecDeque};

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection};

use crate::{
    components::field_source::FieldSourcesComponent, entity_def, entity_index::EntityIndex,
    fields::FieldGrid, map::Map, session::player_id::PlayerId,
};
use ferrets_content::{
    field::{FieldAction, FieldDef, FieldId},
    registry::ContentRegistry,
};

/// Covers or clears `field` around the cell `center` for `player`: the
/// one-cell [`apply_action_around`].
///
/// Covered cells have no source sustaining them, so they live by the field's
/// decay policy from the next recompute on; clearing removes only what no
/// source sustained at the last recompute.
pub fn apply_action(
    world: &mut World,
    player: PlayerId,
    field: FieldId,
    center: CellPos,
    reach: u32,
    action: FieldAction,
) {
    apply_action_around(world, player, field, CellRect::cell(center), reach, action);
}

/// Covers or clears `field` within `reach` of `footprint`. A cover spreads
/// from the footprint through cells on the field's terrains, for `player`;
/// a clear takes the whole disc, walls and all, and drops whatever cover in it
/// no source sustains, whoever's it is.
pub(crate) fn apply_action_around(
    world: &mut World,
    player: PlayerId,
    field: FieldId,
    footprint: CellRect,
    reach: u32,
    action: FieldAction,
) {
    let def = *world.resource::<ContentRegistry>().field_def(field);
    let map = world.resource::<Map>();
    let cells = match action {
        FieldAction::Cover => flood(map, &def, footprint, reach),
        FieldAction::Clear => projection::circle_cells(footprint, reach, map.size()),
    };
    let mut grid = world.resource_mut::<FieldGrid>();
    for cell in cells {
        match action {
            FieldAction::Cover => grid.cover(field, cell, player),
            FieldAction::Clear => grid.clear_unsustained(field, cell),
        }
    }
}

/// Hands the field `entity`'s sources hold around it to `to`, its new owner,
/// from the owner it had: every cell within a source's reach
/// (`FieldSourceState::reach`) that the old owner covers becomes `to`'s, and
/// stays the old owner's too where it lies within the reach of another of
/// its sources of that field, working or not. A field whose reach the old
/// owner may have lost marks `to` as well. A source reaching nothing, an
/// owner-less body, or a body off the map has none to hand over.
pub(crate) fn reassign_cover(world: &mut World, entity: Entity, to: PlayerId) {
    if entity_def::hidden(world, entity) {
        return;
    }
    let Some(from) = entity_def::owner(world, entity) else {
        return;
    };
    let Some(states) = world.get::<FieldSourcesComponent>(entity) else {
        return;
    };
    let footprint = entity_def::location(world, entity).footprint();
    let (map, registry) = (world.resource::<Map>(), world.resource::<ContentRegistry>());
    let handed: Vec<(FieldId, Vec<CellPos>, HashSet<CellPos>)> = states
        .0
        .iter()
        .filter_map(|state| {
            let reach = state.reach()?;
            let def = registry.field_def(state.field);
            Some((
                state.field,
                flood(map, def, footprint, reach),
                held_by_others(world, entity, from, state.field, disc_box(footprint, reach)),
            ))
        })
        .collect();
    let mut grid = world.resource_mut::<FieldGrid>();
    let carried: Vec<FieldId> = handed
        .iter()
        .map(|&(field, _, _)| field)
        .filter(|&field| grid.lost_by(field).contains(from))
        .collect();
    for field in carried {
        grid.mark_lost_by(field, to);
    }
    for (field, cells, kept) in handed {
        for cell in cells {
            if kept.contains(&cell) {
                grid.share(field, cell, from, to);
            } else {
                grid.reassign(field, cell, from, to);
            }
        }
    }
}

/// The cells on the field's terrains within `reach` of `footprint` and
/// connected to it through such cells, in the order a breadth-first walk
/// from the footprint visits them. The walk starts from every cell of the
/// footprint, whatever its terrain; a footprint cell the field's terrains
/// reject is walked from but not covered.
pub(crate) fn flood(map: &Map, def: &FieldDef, footprint: CellRect, reach: u32) -> Vec<CellPos> {
    let (width, height) = (map.width(), map.height());
    // The walk stays inside the disc's box, cut to the map.
    let disc = disc_box(footprint, reach);
    let low = disc.origin;
    let span = CellSize::new(
        (disc.origin.x + disc.size.width).min(width) - low.x,
        (disc.origin.y + disc.size.height).min(height) - low.y,
    );
    let mut visited = vec![false; (span.width * span.height) as usize];
    let index = |cell: CellPos| ((cell.y - low.y) * span.width + (cell.x - low.x)) as usize;
    let mut queue: VecDeque<CellPos> = VecDeque::new();
    let mut out = Vec::new();
    let lies_on = |cell: CellPos| {
        map.nav_grid()
            .terrain(cell)
            .is_some_and(|terrain| def.terrains().admits(terrain))
    };

    for cell in footprint.cells().filter(|&cell| map.contains(cell)) {
        visited[index(cell)] = true;
        queue.push_back(cell);
        if lies_on(cell) {
            out.push(cell);
        }
    }

    while let Some(cell) = queue.pop_front() {
        let neighbors = [
            (cell.x > 0).then(|| CellPos::new(cell.x - 1, cell.y)),
            (cell.x + 1 < width).then(|| CellPos::new(cell.x + 1, cell.y)),
            (cell.y > 0).then(|| CellPos::new(cell.x, cell.y - 1)),
            (cell.y + 1 < height).then(|| CellPos::new(cell.x, cell.y + 1)),
        ];
        for next in neighbors.into_iter().flatten() {
            if !projection::in_circle(next, footprint, reach) {
                continue;
            }
            if visited[index(next)] {
                continue;
            }
            visited[index(next)] = true;
            if !lies_on(next) {
                continue;
            }
            queue.push_back(next);
            out.push(next);
        }
    }
    out
}

/// The cells of `field` within `bounds` that lie within the reach
/// (`FieldSourceState::reach`) of `player`'s sources other than `entity`'s.
fn held_by_others(
    world: &World,
    entity: Entity,
    player: PlayerId,
    field: FieldId,
    bounds: CellRect,
) -> HashSet<CellPos> {
    let (map, registry) = (world.resource::<Map>(), world.resource::<ContentRegistry>());
    let def = registry.field_def(field);
    let mut held = HashSet::new();
    for (_, other) in world.resource::<EntityIndex>().alive_entries() {
        if other == entity
            || entity_def::hidden(world, other)
            || entity_def::owner(world, other) != Some(player)
        {
            continue;
        }
        let Some(states) = world.get::<FieldSourcesComponent>(other) else {
            continue;
        };
        let footprint = entity_def::location(world, other).footprint();
        for state in states.0.iter().filter(|state| state.field == field) {
            let Some(reach) = state.reach() else {
                continue;
            };
            if disc_box(footprint, reach).intersects(bounds) {
                held.extend(flood(map, def, footprint, reach));
            }
        }
    }
    held
}

/// The rectangle a disc of `reach` around `footprint` lies within.
fn disc_box(footprint: CellRect, reach: u32) -> CellRect {
    let origin = CellPos::new(
        footprint.origin.x.saturating_sub(reach),
        footprint.origin.y.saturating_sub(reach),
    );
    let far_x = footprint
        .origin
        .x
        .saturating_add(footprint.size.width)
        .saturating_add(reach);
    let far_y = footprint
        .origin
        .y
        .saturating_add(footprint.size.height)
        .saturating_add(reach);
    CellRect::new(origin, CellSize::new(far_x - origin.x, far_y - origin.y))
}
