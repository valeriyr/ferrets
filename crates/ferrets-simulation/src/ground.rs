//! Whether the ground takes a footprint: free on the navigation grid, and
//! clear of what stands underfoot there without claiming its cells.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::location::LocationDef;
use ferrets_geometry::cell_rect::CellRect;
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_physics::body;

use crate::{
    components::location::LocationComponent,
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    occupancy::{GridMark, OccupancyClass},
};

/// A body standing on the map that holds its ground without claiming its
/// cells ([`Solidity::Underfoot`](ferrets_content::location::Solidity::Underfoot)):
/// a mover crosses it, a static footprint is not raised over it.
pub(crate) struct UnderfootBody {
    /// The layers it stands on.
    layers: LayerMask,
    /// The cells it covers.
    footprint: CellRect,
}

/// Whether the ground takes a footprint of `to` at `placed` on the plane
/// `occupancy` selects: every cell free on the grid but for what `lifted`
/// marks — what the bodies being placed mark on it already (see
/// [`entity_def::grid_mark`]) — and nothing standing underfoot in it but
/// `ignored`, those bodies themselves.
pub fn can_place(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    ignored: &[Entity],
) -> bool {
    on_grid(world, placed, to, lifted)
        && match occupancy {
            OccupancyClass::Claim => true,
            OccupancyClass::Static => is_clear(&standing_underfoot(world, ignored), placed, to),
        }
}

/// [`can_place`] with what stands underfoot already read: `underfoot`, from
/// [`standing_underfoot`], read only for a footprint on the static plane.
pub(crate) fn can_place_clear_of(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    underfoot: &[UnderfootBody],
) -> bool {
    on_grid(world, placed, to, lifted)
        && match occupancy {
            OccupancyClass::Claim => true,
            OccupancyClass::Static => is_clear(underfoot, placed, to),
        }
}

/// Every body standing underfoot now, none of `ignored` counted.
pub(crate) fn standing_underfoot(world: &World, ignored: &[Entity]) -> Vec<UnderfootBody> {
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter_map(|(_, entity)| {
            // An entity off the map holds no ground and stands nowhere.
            if ignored.contains(&entity) || entity_def::hidden(world, entity) {
                return None;
            }
            let standing_def = entity_def::of(world, entity);
            let standing_location = standing_def.location?;
            // What claims its cells is the grid's to refuse; what neither
            // claims nor holds ground stops nothing.
            if standing_location.solidity().claims_cells()
                || !standing_location.solidity().holds_ground()
            {
                return None;
            }
            Some(UnderfootBody {
                layers: standing_location.occupation(),
                footprint: entity_def::footprint_rect_of(world, standing_def, entity),
            })
        })
        .collect()
}

/// Whether every cell of a footprint of `to` at `placed` is free on the grid
/// but for what `lifted` marks.
fn on_grid(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    lifted: &[GridMark],
) -> bool {
    world
        .resource::<Map>()
        .can_place_entity_over_lifted(placed, &to, lifted)
}

/// Whether none of `underfoot` stands on the ground a static footprint of
/// `to` at `placed` would be raised over.
fn is_clear(underfoot: &[UnderfootBody], placed: &LocationComponent, to: LocationDef) -> bool {
    let footprint = CellRect::new(body::anchor(placed.position), to.size());
    !underfoot.iter().any(|body| {
        body.layers & to.occupation() != LayerMask::EMPTY && body.footprint.intersects(footprint)
    })
}
