//! The debug-build check that the navigation grid's claim plane matches the
//! bodies on it.

use std::collections::BTreeMap;

use bevy_ecs::world::World;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_nav_grid::{layer_id::LayerId, layer_mask::LayerMask, nav_grid::NavGrid};

use crate::{
    components::{location::LocationComponent, morph::MorphComponent},
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    occupancy::OccupancyClass,
};

/// Asserts, in debug builds, that the claim plane holds exactly the claims of
/// the bodies standing on the grid and the reservations of changes under way,
/// and under the cell model at most one claim per cell and layer.
pub fn check(world: &mut World) {
    debug_assert!(
        claims_match_bodies(world),
        "the claim plane holds exactly the claims of the bodies standing on the grid \
         and the reservations of changes under way"
    );
    debug_assert!(
        match world.resource::<Map>().movement_model() {
            MovementModel::Cell => claims_at_most_one(world.resource::<Map>().nav_grid()),
            MovementModel::Continuous => true,
        },
        "a cell holds one claim per layer under the cell model"
    );
}

/// Whether every cell's claim count on every layer is the number of claims
/// the bodies standing on the grid mark there, dying ones included, plus the
/// reservations of changes under way.
fn claims_match_bodies(world: &mut World) -> bool {
    let mut expected: BTreeMap<(LayerId, CellPos), u16> = BTreeMap::new();
    let mut count = |layers: LayerMask, cell: CellPos| {
        for layer in layers.layers() {
            *expected.entry((layer, cell)).or_insert(0) += 1;
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
                    count(mark.layers, cell);
                }
            }
            OccupancyClass::Static => {}
        }
    }
    // A raw query is fine here: counts add up the same in any order.
    for morph in world.query::<&MorphComponent>().iter(world) {
        if let Some(reservation) = morph
            .as_under_way()
            .and_then(|under_way| under_way.reservation.as_ref())
        {
            for &(cell, layers) in &reservation.claims {
                count(layers, cell);
            }
        }
    }
    // Each expected count is on the grid, and the grid holds no claim
    // beyond them: its counts add up to theirs.
    let grid = world.resource::<Map>().nav_grid();
    let held: u64 = counts(grid).map(u64::from).sum();
    expected
        .iter()
        .all(|(&(layer, cell), &count)| grid.claim_count(layer, cell) == count)
        && held
            == expected
                .values()
                .map(|&count| u64::from(count))
                .sum::<u64>()
}

/// Whether no cell holds more than one claim on any layer.
fn claims_at_most_one(grid: &NavGrid) -> bool {
    counts(grid).all(|count| count <= 1)
}

/// The claim count of every cell on every registered layer.
fn counts(grid: &NavGrid) -> impl Iterator<Item = u16> + '_ {
    grid.layers().layers().flat_map(move |layer| {
        (0..grid.height()).flat_map(move |y| {
            (0..grid.width()).map(move |x| grid.claim_count(layer, CellPos::new(x, y)))
        })
    })
}
