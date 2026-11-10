//! Whether the ground takes a footprint: ground its shape may stand on, free
//! on the navigation grid, and clear of what holds the ground there without
//! claiming its cells.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    location::{LocationDef, Solidity},
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing, layer_mask::LayerMask, mover_shape::MoverShape,
};

use crate::{
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    occupancy::{GridMark, OccupancyClass},
    simulation_id::SimulationId,
};

/// A body standing on the map that keeps static footprints off its cells
/// though it claims none.
pub(crate) struct GroundHolder {
    /// The layers it stands on.
    layers: LayerMask,
    /// The cells it keeps static footprints off.
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

/// Whether the ground takes a footprint of `to` anchored at `anchor` on the
/// plane `occupancy` selects: ground it [`takes`]; every cell free of standing
/// footprints but for what `lifted` marks — what the bodies being placed
/// mark on it already (see [`entity_def::grid_mark`]) — and, for a body
/// claiming its cells, clear of units and reservations but those of
/// `lifted`'s holders, each measured as the plane measures it (see
/// [`entity_def::measured_rect`]); and no ground holder keeping it off but
/// `ignored`, those bodies themselves.
pub fn can_place(
    world: &World,
    anchor: CellPos,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    ignored: &[Entity],
) -> bool {
    on_free_ground(world, anchor, to, occupancy, lifted)
        && match occupancy {
            OccupancyClass::Claim => true,
            OccupancyClass::Static => is_clear(&ground_holders(world, ignored), anchor, to),
        }
}

/// [`can_place`] with the ground holders already read: `holders`, from
/// [`ground_holders`], read only for a footprint on the static plane.
pub(crate) fn can_place_clear_of(
    world: &World,
    anchor: CellPos,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
    holders: &[GroundHolder],
) -> bool {
    on_free_ground(world, anchor, to, occupancy, lifted)
        && match occupancy {
            OccupancyClass::Claim => true,
            OccupancyClass::Static => is_clear(holders, anchor, to),
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
            // What claims its cells is judged in `clear_of_units`; a mover
            // claiming none keeps solid static footprints off every cell its
            // body lies over; a body that never walks keeps off what its
            // solidity says.
            let (footprint, keeps) = match (standing_location.solidity(), standing_def.can_move()) {
                (Solidity::Solid, _) => return None,
                (Solidity::Underfoot | Solidity::Passable, true) => {
                    (entity_def::covered_rect(world, entity), Keeps::Solid)
                }
                (solidity @ (Solidity::Underfoot | Solidity::Passable), false) => (
                    entity_def::location(world, entity).footprint(),
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

/// Whether the ground [`takes`] a footprint of `to` anchored at `anchor` on the
/// plane `occupancy` selects, every cell free of standing footprints but for
/// what `lifted` marks, and, for a body claiming its cells, clear of units and
/// reservations (see [`clear_of_units`]).
fn on_free_ground(
    world: &World,
    anchor: CellPos,
    to: LocationDef,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
) -> bool {
    let map = world.resource::<Map>();
    let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &to);
    takes(map, anchor, shape, occupancy)
        && map.is_statically_free_over_lifted(anchor, shape, lifted)
        && match to.solidity() {
            Solidity::Solid => clear_of_units(
                world,
                CellRect::new(anchor, shape.size),
                shape.layers,
                occupancy,
                lifted,
            ),
            Solidity::Underfoot | Solidity::Passable => true,
        }
}

/// Whether no unit stands over `footprint` on any of `layers`, measured as a
/// placement on the plane `occupancy` selects measures it (see
/// [`entity_def::measured_rect`]), and no reservation holds a cell of it
/// there, but those of the holders of `lifted`'s claims. The units are the
/// holders of the claims over the footprint and the ring of cells around
/// it.
fn clear_of_units(
    world: &World,
    footprint: CellRect,
    layers: LayerMask,
    occupancy: OccupancyClass,
    lifted: &[GridMark],
) -> bool {
    let grid = world.resource::<Map>().nav_grid();
    let around = CellRect::new(
        CellPos::new(
            footprint.origin.x.saturating_sub(1),
            footprint.origin.y.saturating_sub(1),
        ),
        CellSize::new(footprint.size.width + 2, footprint.size.height + 2),
    );
    let mut holders: Vec<u32> = around
        .cells()
        .flat_map(|cell| {
            layers
                .layers()
                .flat_map(move |layer| grid.claim_holders(layer, cell).iter().copied())
        })
        .collect();
    holders.sort_unstable();
    holders.dedup();
    holders
        .into_iter()
        .map(SimulationId)
        .filter(|&holder| {
            !lifted.iter().any(|mark| match mark.class {
                OccupancyClass::Claim => mark.holder == holder,
                OccupancyClass::Static => false,
            })
        })
        .all(|holder| {
            let body = world
                .resource::<EntityIndex>()
                .any(holder)
                .expect("a claim's holder is a simulation entity");
            // A holder standing on the static plane claims cells only through
            // a reservation, read below.
            let def = entity_def::of(world, body);
            let lies_over = match OccupancyClass::of(def) {
                OccupancyClass::Static => false,
                OccupancyClass::Claim => {
                    def.location
                        .expect("a claim's holder stands somewhere")
                        .occupies()
                        & layers
                        != LayerMask::EMPTY
                        && entity_def::measured_rect(world, body, occupancy).intersects(footprint)
                }
            };
            let reserves = entity_def::reservation(world, body).is_some_and(|reservation| {
                reservation.marks().any(|mark| {
                    mark.layers & layers != LayerMask::EMPTY && mark.footprint.intersects(footprint)
                })
            });
            !lies_over && !reserves
        })
}

/// Whether none of `holders` keeps a static footprint of `to` anchored at
/// `anchor` off the ground it would be raised over.
fn is_clear(holders: &[GroundHolder], anchor: CellPos, to: LocationDef) -> bool {
    let footprint = CellRect::new(anchor, to.size());
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
