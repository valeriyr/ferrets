//! Ground a body holds ahead of standing on it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{
    entity_def,
    map::Map,
    occupancy::{GridMark, OccupancyClass},
    simulation_id::SimulationId,
};

/// Ground held for a body ahead of its standing on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    /// The body the ground is held for: the holder of its claims.
    holder: SimulationId,
    /// The footprint held.
    footprint: CellRect,
    /// The layers held on it.
    layers: LayerMask,
    /// The holder's own mark when the ground was reserved: on the layers its
    /// claim holds, the footprint is held around the claim's cells.
    standing: Option<GridMark>,
}

impl Reservation {
    /// The claims the reservation holds: the footprint on every layer the
    /// holder's standing claim leaves free, and the footprint less that
    /// claim's cells on the layers it holds.
    pub(crate) fn marks(&self) -> impl Iterator<Item = GridMark> {
        let reservation = *self;
        let cut = match reservation.standing {
            Some(mark) => match mark.class {
                OccupancyClass::Claim => Some((mark.footprint, mark.layers & reservation.layers)),
                OccupancyClass::Static => None,
            },
            None => None,
        };
        let shared = cut.map_or(LayerMask::EMPTY, |(_, shared)| shared);
        let around = cut.into_iter().flat_map(move |(standing, shared)| {
            reservation
                .footprint
                .minus(standing)
                .map(move |rect| (rect, shared))
        });
        std::iter::once((reservation.footprint, reservation.layers & !shared))
            .chain(around)
            .filter(|&(_, layers)| layers != LayerMask::EMPTY)
            .map(move |(footprint, layers)| GridMark {
                holder: reservation.holder,
                footprint,
                layers,
                class: OccupancyClass::Claim,
            })
    }
}

/// Holds for `entity` the `size` footprint at `origin` on `layers`, around
/// the cells its own standing claim already holds.
///
/// Expects the cells held unclaimed; panics in debug builds otherwise.
pub(crate) fn reserve(
    world: &mut World,
    entity: Entity,
    layers: LayerMask,
    origin: CellPos,
    size: CellSize,
) -> Reservation {
    let reservation = Reservation {
        holder: entity_def::simulation_id(world, entity),
        footprint: CellRect::new(origin, size),
        layers,
        standing: entity_def::grid_mark(world, entity),
    };
    let mut map = world.resource_mut::<Map>();
    debug_assert!(
        reservation.marks().all(|mark| mark
            .footprint
            .cells()
            .all(|cell| !map.nav_grid().is_claimed(mark.layers, cell))),
        "a reservation finds a cell claimed only under the reserver's own standing claim"
    );
    for mark in reservation.marks() {
        map.claim(mark);
    }
    reservation
}

/// Lets go of the ground `reservation` holds: the claims it took, wherever
/// its holder has gone since.
pub(crate) fn release(world: &mut World, reservation: &Reservation) {
    let mut map = world.resource_mut::<Map>();
    for mark in reservation.marks() {
        map.unclaim(mark);
    }
}
