//! Contact resolution for the continuous movement model: gathers the
//! bodies, lets the physics separate them, commits the displacements
//! through terrain, and re-derives the claim plane from where they settled.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::registry::ContentRegistry;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_nav_grid::{layer_mask::LayerMask, mover_shape::MoverShape};
use ferrets_physics::{body, body::Body, contact, terrain};

use crate::{
    components::{location::LocationComponent, morph::MorphComponent, movement::MoveComponent},
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
};

/// Resolves body contact between continuous movers: gather every body,
/// take one pass of pairwise separations, and commit each displacement
/// through terrain checks — then rebuild the claim plane from where the
/// bodies settled. A no-op under the cell model.
pub fn resolve(world: &mut World) {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => return,
        MovementModel::Continuous => {}
    }

    // Gather, in simulation-id order.
    let claimants = claimants(world);
    let mut bodies: Vec<Body> = Vec::new();

    for &(entity, shape) in &claimants {
        let position = entity_def::position(world, entity);
        let heading = world
            .entity(entity)
            .get::<MoveComponent>()
            .and_then(|movement| movement.path.last())
            .map(|waypoint| {
                let waypoint = FixedUVec2::from(*waypoint);
                FixedVec2::new(
                    waypoint.x.to_num::<FixedI64>() - position.x.to_num::<FixedI64>(),
                    waypoint.y.to_num::<FixedI64>() - position.y.to_num::<FixedI64>(),
                )
            });
        bodies.push(Body {
            position,
            size: shape.size,
            radius: entity_def::radius(world, entity),
            weight: entity_def::weight(world, entity),
            layers: shape.layers,
            heading,
        });
    }

    // Commit through terrain: a push may never overlap a body onto a
    // statically blocked cell — the blocked axis is dropped, sliding along
    // the other.
    let pushes = contact::separations(&bodies);
    for ((&(entity, shape), body), push) in claimants.iter().zip(&bodies).zip(pushes) {
        // A mover still rising pushes the others but stands where it was set
        // down, as a structure does, until it has risen.
        if push == FixedVec2::ZERO || entity_def::rising(world, entity) {
            continue;
        }
        let desired = terrain::displaced(body.position, push.x, push.y);
        let committed = terrain::slide_toward(
            world.resource::<Map>().nav_grid(),
            shape,
            body.position,
            desired,
            body.radius,
        );
        if committed != body.position {
            debug_assert!(
                {
                    let hierarchy = world.resource::<Map>().hierarchy();
                    hierarchy.is_dirty()
                        || hierarchy
                            .region_of(body::anchor(committed), shape)
                            .is_some()
                },
                "a slid walker keeps an anchor in a region of its shape"
            );
            world
                .entity_mut(entity)
                .get_mut::<LocationComponent>()
                .unwrap()
                .position = committed;
        }
    }

    rebuild_from(world, &claimants);
}

/// Re-derives the claim plane under the continuous model from where its
/// claimants stand now.
///
/// Panics under the cell model, whose claims are never rebuilt.
pub(crate) fn rebuild_claims(world: &mut World) {
    let claimants = claimants(world);
    rebuild_from(world, &claimants);
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

/// Rebuilds the claim plane from where `claimants` stand and from the ground
/// in-flight form changes reserve.
fn rebuild_from(world: &mut World, claimants: &[(Entity, MoverShape)]) {
    // The footprint under each settled body — anchored at the cell the eye
    // puts it on, and as wide as the body moves. Placement and spawning see
    // each unit where it stands (a pushed body's claim follows it), and a
    // walk claims one stepping footprint at a time, as under the cell model.
    // The claim plane is a cell-resolution summary, not the collision
    // geometry — contact and terrain stay with the circles.
    let footprints: Vec<(LayerMask, CellSize, CellPos)> = claimants
        .iter()
        .map(|&(entity, shape)| {
            let position = entity_def::position(world, entity);
            (shape.layers, shape.size, body::anchor(position))
        })
        .collect();

    // Ground reserved by in-flight form changes is claim-plane state too, so
    // the rebuild must re-assert it or a reservation would evaporate one tick
    // after it was made. A raw query is fine here: claims OR together, so the
    // iteration order cannot decide anything.
    let reservations: Vec<(LayerMask, Vec<CellPos>)> = world
        .query::<&MorphComponent>()
        .iter(world)
        .filter_map(|morph| morph.reservation.as_ref())
        .map(|reservation| (reservation.layers, reservation.cells.clone()))
        .collect();
    world
        .resource_mut::<Map>()
        .rebuild_claims(&footprints, &reservations);
}
