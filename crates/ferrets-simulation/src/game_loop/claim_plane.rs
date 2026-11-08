//! The debug-build check that the navigation grid's claim plane matches the
//! bodies on it.

use std::collections::BTreeMap;

use bevy_ecs::world::World;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_nav_grid::{layer_id::LayerId, layer_mask::LayerMask, nav_grid::NavGrid};

use crate::{
    components::{
        entity_info::EntityInfoComponent, location::LocationComponent, morph::MorphComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    occupancy::OccupancyClass,
    simulation_id::SimulationId,
};

/// Asserts, in debug builds, that the claim plane holds exactly the claims of
/// the bodies standing on the grid and the reservations of changes under way,
/// each under its holder, and under the cell model at most one claim per cell
/// and layer.
pub fn check(world: &mut World) {
    debug_assert!(
        claims_match_bodies(world),
        "the claim plane holds exactly the claims of the bodies standing on the grid \
         and the reservations of changes under way, each under its holder"
    );
    debug_assert!(
        match world.resource::<Map>().movement_model() {
            MovementModel::Cell => claims_at_most_one(world.resource::<Map>().nav_grid()),
            MovementModel::Continuous => true,
        },
        "a cell holds one claim per layer under the cell model"
    );
}

/// Whether every cell holds on every layer exactly the claims the bodies
/// standing on the grid mark there, dying ones included, and the reservations
/// of changes under way, each under its own holder.
fn claims_match_bodies(world: &mut World) -> bool {
    let mut expected: BTreeMap<(LayerId, CellPos), Vec<u32>> = BTreeMap::new();
    let mut hold = |holder: SimulationId, layers: LayerMask, cell: CellPos| {
        for layer in layers.layers() {
            expected.entry((layer, cell)).or_default().push(holder.0);
        }
    };
    for (_, entity) in world.resource::<EntityIndex>().all_entries() {
        if !entity_def::stands_on_grid(world, entity)
            || !world.entity(entity).contains::<LocationComponent>()
        {
            continue;
        }
        let Some(mark) = entity_def::grid_mark(world, entity) else {
            continue;
        };
        match mark.class {
            OccupancyClass::Claim => {
                for cell in mark.footprint.cells() {
                    hold(mark.holder, mark.layers, cell);
                }
            }
            OccupancyClass::Static => {}
        }
    }
    // A raw query is fine here: each holder list is sorted below, whatever
    // order filled it.
    for (info, morph) in world
        .query::<(&EntityInfoComponent, &MorphComponent)>()
        .iter(world)
    {
        if let Some(reservation) = morph
            .as_under_way()
            .and_then(|under_way| under_way.reservation.as_ref())
        {
            for mark in reservation.marks() {
                for cell in mark.footprint.cells() {
                    hold(info.id(), mark.layers, cell);
                }
            }
        }
    }
    for holders in expected.values_mut() {
        holders.sort_unstable();
    }
    // Each expected list is on the grid, and the grid holds no claim beyond
    // them: its holders add up to theirs.
    let grid = world.resource::<Map>().nav_grid();
    let held: usize = holder_counts(grid).sum();
    expected
        .iter()
        .all(|(&(layer, cell), holders)| grid.claim_holders(layer, cell) == holders.as_slice())
        && held == expected.values().map(Vec::len).sum::<usize>()
}

/// Whether no cell holds more than one claim on any layer.
fn claims_at_most_one(grid: &NavGrid) -> bool {
    holder_counts(grid).all(|count| count <= 1)
}

/// How many holders claim each cell on each registered layer.
fn holder_counts(grid: &NavGrid) -> impl Iterator<Item = usize> + '_ {
    grid.layers().layers().flat_map(move |layer| {
        (0..grid.height()).flat_map(move |y| {
            (0..grid.width()).map(move |x| grid.claim_holders(layer, CellPos::new(x, y)).len())
        })
    })
}
