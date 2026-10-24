//! Whether the ground takes a footprint: ground its shape may stand on, free
//! on the navigation grid, and clear of what stands underfoot there without
//! claiming its cells.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{location::LocationDef, registry::ContentRegistry};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_pathfinder::{
    cliff_crossing::CliffCrossing, layer_mask::LayerMask, mover_shape::MoverShape,
};
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

/// Whether the ground at `anchor` takes a footprint of `shape` on the plane
/// `occupancy` selects, whoever stands there: every cell ground the shape may
/// stand on, the shape crossing between its own cells, and a static footprint
/// on one level and no slope where its domain's layer spans levels.
pub fn takes(map: &Map, anchor: CellPos, shape: MoverShape, occupancy: OccupancyClass) -> bool {
    let grid = map.nav_grid();
    let flat = match (occupancy, shape.cliff_crossing) {
        (OccupancyClass::Static, Some(CliffCrossing::Ramp | CliffCrossing::Cross)) => {
            grid.is_flat(anchor, shape)
        }
        (OccupancyClass::Static, None) | (OccupancyClass::Claim, _) => true,
    };
    grid.fits_ground(anchor, shape) && grid.holds_together(anchor, shape) && flat
}

/// Whether the ground takes a footprint of `to` at `placed` on the plane
/// `occupancy` selects: ground it [`takes`], every cell free on the grid but
/// for what `lifted` marks — what the bodies being placed mark on it already
/// (see [`entity_def::grid_mark`]) — and nothing standing underfoot in it but
/// `ignored`, those bodies themselves.
pub fn can_place(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    ignored: &[Entity],
) -> bool {
    on_free_ground(world, placed, to, occupancy, lifted)
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
    on_free_ground(world, placed, to, occupancy, lifted)
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
                layers: standing_location.occupies(),
                footprint: entity_def::footprint_rect_of(world, standing_def, entity),
            })
        })
        .collect()
}

/// Whether the ground [`takes`] a footprint of `to` at `placed` on the plane
/// `occupancy` selects, every cell free on the grid but for what `lifted`
/// marks.
fn on_free_ground(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
) -> bool {
    let map = world.resource::<Map>();
    let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &to);
    takes(map, body::anchor(placed.position), shape, occupancy)
        && map.is_free_over_lifted(placed, shape, lifted)
}

/// Whether none of `underfoot` stands on the ground a static footprint of
/// `to` at `placed` would be raised over.
fn is_clear(underfoot: &[UnderfootBody], placed: &LocationComponent, to: LocationDef) -> bool {
    let footprint = CellRect::new(body::anchor(placed.position), to.size());
    !underfoot.iter().any(|body| {
        body.layers & to.occupies() != LayerMask::EMPTY && body.footprint.intersects(footprint)
    })
}
