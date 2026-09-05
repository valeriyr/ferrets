//! Per-tick field recompute: grows every standing source's reach, re-derives
//! what the sources sustain, and lets unsustained coverage decay by each
//! field's policy. Also the one-off acts on a field a cast or a standing
//! entity performs.

use std::collections::VecDeque;

use bevy_ecs::world::World;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection};

use crate::{
    components::{
        build::UnderConstructionComponent, field_source::FieldSourcesComponent,
        hidden::HiddenComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    fields::FieldGrid,
    map::Map,
    session::player_id::PlayerId,
};
use ferrets_content::{
    field::{FieldAction, FieldDecay, FieldDef, FieldGrowth, FieldId},
    registry::ContentRegistry,
};

/// One source's projection this tick.
struct Emission {
    /// The field projected.
    field: FieldId,
    /// The player whose cover it is.
    player: PlayerId,
    /// The footprint it spreads from.
    footprint: CellRect,
    /// How far from the footprint it reaches, in cells.
    reach: u32,
}

/// Recomputes every field for the current tick.
///
/// Sources are read from the alive index, so a dying source stops projecting
/// the tick it dies. Under construction a source projects its
/// `while_constructing` radius, if any, and does not grow. Standing, a gradual
/// source grows one cell per cycle up to its radius. Sustained cells are
/// re-derived from scratch, covered cells absorb them, and the rest decays per
/// field: at once, from the edge inward every cycle, or never.
///
/// Every fold is either commutative or reads a snapshot taken before the pass
/// writes, so source order cannot reach the grid.
pub fn recompute_fields(world: &mut World) {
    let mut emissions: Vec<Emission> = Vec::new();

    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<HiddenComponent>() {
            continue;
        }
        let def = entity_def::of(world, entity);
        if def.field_sources.is_empty() {
            continue;
        }
        let Some(player) = entity_def::owner(world, entity) else {
            continue;
        };
        let footprint = entity_def::occupied_rect(world, entity);
        let constructing = entity_ref.contains::<UnderConstructionComponent>();
        let sources = def.field_sources.clone();

        let mut entity_mut = world.entity_mut(entity);
        let mut component = entity_mut
            .get_mut::<FieldSourcesComponent>()
            .expect("a type with field sources carries their state");
        for (source, state) in sources.iter().zip(component.0.iter_mut()) {
            let reach = if constructing {
                match source.while_constructing() {
                    Some(radius) => radius,
                    None => continue,
                }
            } else {
                match source.growth() {
                    FieldGrowth::Instant => source.radius(),
                    FieldGrowth::Gradual { cycle, .. } => {
                        // A patch projected while constructing is kept rather
                        // than shrunk when the source stands.
                        if let Some(radius) = source.while_constructing()
                            && state.reach < radius
                        {
                            state.reach = radius;
                        }
                        if state.reach < source.radius() {
                            state.countdown -= 1;
                            if state.countdown == 0 {
                                state.reach += 1;
                                state.countdown = cycle;
                            }
                        }
                        state.reach
                    }
                }
            };
            emissions.push(Emission {
                field: source.field(),
                player,
                footprint,
                reach,
            });
        }
    }

    // Cells are derived against the map first; the grid is written after, so
    // the two resources are never borrowed together.
    let fields: Vec<(FieldId, FieldDef)> = {
        let registry = world.resource::<ContentRegistry>();
        registry
            .field_ids()
            .map(|id| (id, *registry.field_def(id)))
            .collect()
    };
    let map = world.resource::<Map>();
    let sustained: Vec<(FieldId, PlayerId, Vec<CellPos>)> = emissions
        .iter()
        .map(|emission| {
            let def = &fields[emission.field.index()].1;
            (
                emission.field,
                emission.player,
                flood(map, def, emission.footprint, emission.reach),
            )
        })
        .collect();

    let mut grid = world.resource_mut::<FieldGrid>();
    for &(field, def) in &fields {
        grid.reset_sustained(field);
        for (_, player, cells) in sustained.iter().filter(|(f, _, _)| *f == field) {
            for &cell in cells {
                grid.sustain(field, cell, *player);
            }
        }
        grid.grow(field);
        match def.decay() {
            FieldDecay::Instant => grid.clear_all_unsustained(field),
            FieldDecay::Never => {}
            FieldDecay::Gradual { cycle } => {
                if grid.decay_due(field, cycle) {
                    grid.recede(field);
                }
            }
        }
    }
}

/// Covers or clears `field` around `center` for `player`, as a cast does.
///
/// Covered cells have no source sustaining them, so they live by the field's
/// decay policy from the next recompute on; clearing removes only what no
/// source sustained at the last recompute.
pub fn apply_action(
    world: &mut World,
    player: PlayerId,
    field: FieldId,
    center: CellPos,
    radius: u32,
    action: FieldAction,
) {
    apply_action_around(
        world,
        player,
        field,
        CellRect::new(center, CellSize::ONE),
        radius,
        action,
    );
}

/// Covers or clears `field` within `radius` of `footprint`. A cover spreads
/// from the footprint through terrain the field's layer passes, for `player`;
/// a clear takes the whole disc, walls and all, and drops whatever cover in it
/// no source sustains, whoever's it is.
pub fn apply_action_around(
    world: &mut World,
    player: PlayerId,
    field: FieldId,
    footprint: CellRect,
    radius: u32,
    action: FieldAction,
) {
    let def = *world.resource::<ContentRegistry>().field_def(field);
    let map = world.resource::<Map>();
    let cells = match action {
        FieldAction::Cover => flood(map, &def, footprint, radius),
        FieldAction::Clear => projection::circle_cells(footprint, radius)
            .into_iter()
            .filter(|&cell| map.contains(cell))
            .collect(),
    };
    let mut grid = world.resource_mut::<FieldGrid>();
    for cell in cells {
        match action {
            FieldAction::Cover => grid.cover(field, cell, player),
            FieldAction::Clear => grid.clear_unsustained(field, cell),
        }
    }
}

/// The cells within `reach` of `footprint` connected to it through cells whose
/// terrain passes the field's layer, in the order a breadth-first walk from the
/// footprint visits them. The footprint's own cells are included whatever
/// their terrain.
fn flood(map: &Map, def: &FieldDef, footprint: CellRect, reach: u32) -> Vec<CellPos> {
    let (width, height) = (map.width(), map.height());
    // The walk never leaves the disc's bounding box, so that is all the
    // bookkeeping it needs.
    let low = CellPos::new(
        footprint.origin.x.saturating_sub(reach),
        footprint.origin.y.saturating_sub(reach),
    );
    let span = CellSize::new(
        (footprint.origin.x + footprint.size.width + reach).min(width) - low.x,
        (footprint.origin.y + footprint.size.height + reach).min(height) - low.y,
    );
    let mut visited = vec![false; (span.width * span.height) as usize];
    let index = |cell: CellPos| ((cell.y - low.y) * span.width + (cell.x - low.x)) as usize;
    let mut queue: VecDeque<CellPos> = VecDeque::new();
    let mut out = Vec::new();

    for cell in footprint.cells().filter(|&cell| map.contains(cell)) {
        visited[index(cell)] = true;
        queue.push_back(cell);
        out.push(cell);
    }

    while let Some(cell) = queue.pop_front() {
        let neighbours = [
            (cell.x > 0).then(|| CellPos::new(cell.x - 1, cell.y)),
            (cell.x + 1 < width).then(|| CellPos::new(cell.x + 1, cell.y)),
            (cell.y > 0).then(|| CellPos::new(cell.x, cell.y - 1)),
            (cell.y + 1 < height).then(|| CellPos::new(cell.x, cell.y + 1)),
        ];
        for next in neighbours.into_iter().flatten() {
            if !projection::in_circle(next, footprint, reach) {
                continue;
            }
            if visited[index(next)] {
                continue;
            }
            visited[index(next)] = true;
            if !map.nav_grid().is_terrain_passable_by(def.layer(), next) {
                continue;
            }
            queue.push_back(next);
            out.push(next);
        }
    }
    out
}
