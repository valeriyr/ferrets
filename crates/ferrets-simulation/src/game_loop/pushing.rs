//! Contact resolution for the continuous movement model: gathers the
//! bodies, lets the physics separate them, commits the displacements
//! through terrain, and re-derives the claim plane from where they settled.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_physics::{body, body::Body, contact, terrain};

use crate::{
    components::{
        hidden::HiddenComponent, location::LocationComponent, morph::MorphComponent,
        movement::MoveComponent,
    },
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
    let mut entities: Vec<Entity> = Vec::new();
    let mut bodies: Vec<Body> = Vec::new();
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if world.entity(entity).contains::<HiddenComponent>() {
            continue;
        }
        let Some(location) = world.entity(entity).get::<LocationComponent>() else {
            continue;
        };
        let position = location.position;
        let def = entity_def::of(world, entity);
        if !def.can_move() {
            continue;
        }
        let Some(location_def) = def.location else {
            continue;
        };
        if !location_def.solidity().claims_cells() {
            continue;
        }
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
        entities.push(entity);
        bodies.push(Body {
            position,
            size: location_def.size(),
            radius: entity_def::radius(world, entity),
            weight: entity_def::weight(world, entity),
            mask: location_def.occupation(),
            heading,
        });
    }

    // Commit through terrain: a push may never overlap a body onto a
    // statically blocked cell — the blocked axis is dropped, sliding along
    // the other.
    let pushes = contact::separations(&bodies);
    for ((&entity, body), push) in entities.iter().zip(&bodies).zip(pushes) {
        if push == FixedVec2::ZERO {
            continue;
        }
        let desired = terrain::displaced(body.position, push.x, push.y);
        let committed = terrain::slide_toward(
            world.resource::<Map>().nav_grid(),
            body.mask,
            body.position,
            body.size,
            desired,
            body.radius,
        );
        if committed != body.position {
            world
                .entity_mut(entity)
                .get_mut::<LocationComponent>()
                .unwrap()
                .position = committed;
        }
    }

    // The footprint under each settled body — anchored at the cell the eye
    // puts it on, and as wide as the body moves. Placement and spawning see
    // each unit where it stands (a pushed body's claim follows it), and a
    // walk claims one stepping footprint at a time, as under the cell model.
    // The claim plane is a cell-resolution summary, not the collision
    // geometry — contact and terrain stay with the circles.
    let footprints: Vec<(LayerMask, CellSize, CellPos)> = entities
        .iter()
        .zip(&bodies)
        .map(|(&entity, body)| {
            let position = world
                .entity(entity)
                .get::<LocationComponent>()
                .unwrap()
                .position;
            (body.mask, body.size, body::anchor(position))
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
        .map(|reservation| (reservation.mask, reservation.cells.clone()))
        .collect();
    world
        .resource_mut::<Map>()
        .rebuild_claims(&footprints, &reservations);
}
