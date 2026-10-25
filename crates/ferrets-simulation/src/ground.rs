//! Whether the ground takes a footprint: ground its shape may stand on, free
//! on the navigation grid, and clear of what holds the ground there without
//! claiming its cells.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    location::{LocationDef, Solidity},
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_nav_grid::{
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

/// A body standing on the map that keeps static footprints off its cells
/// though it claims none.
pub(crate) struct GroundHolder {
    /// The layers it stands on.
    layers: LayerMask,
    /// The cells it holds.
    footprint: CellRect,
    /// Which static footprints it keeps off them.
    keeps: Keeps,
}

/// Which static footprints a ground holder keeps off its cells.
enum Keeps {
    /// Solid ones: a mover, which no solid footprint is raised over.
    Solid,
    /// Those its solidity keeps off: a body that never walks.
    BySolidity(Solidity),
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
/// `occupancy` selects: ground it [`takes`]; every cell free on the grid but
/// for what `lifted` marks — what the bodies being placed mark on it already
/// (see [`entity_def::grid_mark`]) — of claims too for a body claiming its
/// cells, of standing footprints alone for one that claims none; and no
/// ground holder keeping it off but `ignored`, those bodies themselves.
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
            OccupancyClass::Static => is_clear(&ground_holders(world, ignored), placed, to),
        }
}

/// [`can_place`] with the ground holders already read: `holders`, from
/// [`ground_holders`], read only for a footprint on the static plane.
pub(crate) fn can_place_clear_of(
    world: &World,
    placed: &LocationComponent,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    holders: &[GroundHolder],
) -> bool {
    on_free_ground(world, placed, to, occupancy, lifted)
        && match occupancy {
            OccupancyClass::Claim => true,
            OccupancyClass::Static => is_clear(holders, placed, to),
        }
}

/// Every ground holder standing now, alive or dying, remains included, none
/// of `ignored` counted and none whose death has ended.
pub(crate) fn ground_holders(world: &World, ignored: &[Entity]) -> Vec<GroundHolder> {
    world
        .resource::<EntityIndex>()
        .all_entries()
        .into_iter()
        .filter_map(|(_, entity)| {
            // An entity off the map holds no ground and stands nowhere; one
            // seated in a berth sits on its job's ground and leaves it only
            // through a set-down; one whose death has ended leaves its ground
            // to what it leaves behind.
            if ignored.contains(&entity)
                || entity_def::hidden(world, entity)
                || entity_def::attached(world, entity)
                || entity_def::died(world, entity)
            {
                return None;
            }
            let standing_def = entity_def::of(world, entity);
            let standing_location = standing_def.location?;
            // What claims its cells is the grid's to refuse; a mover claiming
            // none keeps solid static footprints off every cell it stands
            // across; a body that never walks keeps off what its solidity says.
            let (footprint, keeps) = match (standing_location.solidity(), standing_def.can_move()) {
                (Solidity::Solid, _) => return None,
                (Solidity::Underfoot | Solidity::Passable, true) => {
                    (entity_def::standing_rect(world, entity), Keeps::Solid)
                }
                (solidity @ (Solidity::Underfoot | Solidity::Passable), false) => (
                    entity_def::footprint_rect_of(world, standing_def, entity),
                    Keeps::BySolidity(solidity),
                ),
            };
            Some(GroundHolder {
                layers: standing_location.occupies(),
                footprint,
                keeps,
            })
        })
        .collect()
}

/// Whether the ground [`takes`] a footprint of `to` at `placed` on the plane
/// `occupancy` selects, every cell free on the grid but for what `lifted`
/// marks: of claims too for a body claiming its cells, of standing footprints
/// alone for one that claims none.
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
        && match to.solidity() {
            Solidity::Solid => map.is_free_over_lifted(placed, shape, lifted),
            Solidity::Underfoot | Solidity::Passable => {
                map.is_statically_free_over_lifted(placed, shape, lifted)
            }
        }
}

/// Whether none of `holders` keeps a static footprint of `to` at `placed` off
/// the ground it would be raised over.
fn is_clear(holders: &[GroundHolder], placed: &LocationComponent, to: LocationDef) -> bool {
    let footprint = CellRect::new(body::anchor(placed.position), to.size());
    !holders.iter().any(|holder| {
        holder.layers & to.occupies() != LayerMask::EMPTY
            && holder.footprint.intersects(footprint)
            && match holder.keeps {
                Keeps::Solid => match to.solidity() {
                    Solidity::Solid => true,
                    Solidity::Underfoot | Solidity::Passable => false,
                },
                Keeps::BySolidity(solidity) => solidity.keeps_off(to.solidity()),
            }
    })
}
