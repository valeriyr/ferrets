//! Contact resolution for the continuous movement model: gathers the
//! bodies, lets the physics separate them, and commits the displacements
//! through terrain, each body's claim following it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::registry::ContentRegistry;
use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};
use ferrets_nav_grid::mover_shape::MoverShape;
use ferrets_physics::{body::Body, contact, terrain};

use crate::{
    components::{location::LocationComponent, movement::MoveComponent},
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::{self, MovementModel},
};

/// Resolves body contact between continuous movers: gather every body,
/// take one pass of pairwise separations, and commit each displacement
/// through terrain checks, each body's claim following it. A no-op under the
/// cell model.
pub fn resolve(world: &mut World) {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => return,
        MovementModel::Continuous => {}
    }

    // Gather, in simulation-id order.
    let claimants = claimants(world);
    let map = world.resource::<Map>();
    let mut bodies: Vec<Body> = Vec::new();

    for &(entity, shape) in &claimants {
        let location = entity_def::location(world, entity);
        let position = location.middle();
        let heading = world
            .entity(entity)
            .get::<MoveComponent>()
            .and_then(|movement| movement.path.last())
            .map(|waypoint| {
                let waypoint = map.location_at(*waypoint, shape.size).middle();
                FixedVec2::new(
                    waypoint.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
                    waypoint.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
                )
            });
        bodies.push(Body {
            location,
            radius: entity_def::radius(world, entity),
            weight: entity_def::weight(world, entity),
            layers: shape.layers,
            heading,
        });
    }

    // Commit through terrain: a push may never overlap a body onto a
    // statically blocked cell — the blocked axis is dropped, sliding along
    // the other — nor leave the footprint it is anchored at anywhere
    // its shape does not fit: past the map's edge, or over standing ground a
    // circle smaller than its footprint can skirt.
    let pushes = contact::separations(&bodies);
    for ((&(entity, shape), body), push) in claimants.iter().zip(&bodies).zip(pushes) {
        // A mover still rising, or changing form on ground it reserved,
        // pushes the others but stands where it is, as a structure does,
        // until it has risen or the change has landed.
        if push == FixedVec2::ZERO
            || entity_def::rising(world, entity)
            || entity_def::reserving(world, entity)
        {
            continue;
        }
        let map = world.resource::<Map>();
        let (position, grid) = (body.location.middle(), map.nav_grid());
        let desired = terrain::displaced(body.location, push.x, push.y, map.size());
        let slid = map.location(
            terrain::slide_toward(grid, shape, position, desired, body.radius),
            shape.size,
        );
        let committed = if grid.fits_statically(slid.anchor(), shape) {
            slid
        } else {
            body.location
        };
        if committed != body.location {
            debug_assert!(
                {
                    let hierarchy = map.hierarchy();
                    hierarchy.is_dirty_for(shape)
                        || hierarchy.region_of(committed.anchor(), shape).is_some()
                },
                "a slid walker keeps an anchor in a region of its shape"
            );
            movement_model::move_body(world, entity, committed);
        }
    }
}

/// Every body standing on the grid that claims its cells as it moves, with
/// the shape it moves as, in simulation-id order.
fn claimants(world: &World) -> Vec<(Entity, MoverShape)> {
    let registry = world.resource::<ContentRegistry>();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter_map(|(_, entity)| {
            if !entity_def::stands_on_grid(world, entity)
                || !world.entity(entity).contains::<LocationComponent>()
            {
                return None;
            }
            let def = entity_def::of(world, entity);
            let location_def = def.location?;
            (def.can_move() && location_def.solidity().claims_cells())
                .then(|| (entity, entity_def::mover_shape(registry, &location_def)))
        })
        .collect()
}
