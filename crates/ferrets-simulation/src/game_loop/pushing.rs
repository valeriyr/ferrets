//! Contact resolution for the continuous movement model: gathers the
//! bodies, lets the physics settle their contacts, and commits each body's
//! new place, its claim following it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::registry::ContentRegistry;
use ferrets_nav_grid::mover_shape::MoverShape;
use ferrets_physics::{body::Body, resolve};

use crate::{
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::{self, MovementModel},
};

/// Resolves body contact between continuous movers: gathers every body with
/// what it is doing, read once, settles their contacts in one pass (see
/// [`resolve::settle`]), and commits each changed place, the body's claim
/// following it. A no-op under the cell model.
pub fn resolve(world: &mut World) {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => return,
        MovementModel::Continuous => {}
    }

    // Gathered in simulation-id order.
    let claimants = claimants(world);
    let bodies: Vec<Body> = claimants
        .iter()
        .map(|&(entity, shape)| Body {
            location: entity_def::location(world, entity),
            radius: entity_def::radius(world, entity),
            weight: entity_def::weight(world, entity),
            mover: shape,
            bearing: entity_def::bearing(world, entity),
        })
        .collect();
    let settled = resolve::settle(
        &bodies,
        |first, second| entity_def::meeting(world, claimants[first].0, claimants[second].0),
        world.resource::<Map>().nav_grid(),
    );

    for ((&(entity, shape), body), committed) in claimants.iter().zip(&bodies).zip(settled) {
        if committed == body.location {
            continue;
        }
        assert!(
            {
                let hierarchy = world.resource::<Map>().hierarchy();
                hierarchy.is_dirty_at(committed.anchor(), shape)
                    || hierarchy.region_of(committed.anchor(), shape).is_some()
            },
            "a body stands on an anchor in a region of its shape, unless this \
             tick's changes left its own cluster stale"
        );
        movement_model::move_body(world, entity, committed);
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
            if !entity_def::stands_on_grid(world, entity) {
                return None;
            }
            let def = entity_def::of(world, entity);
            let form = entity_def::form(world, def);
            (def.can_move() && form.solidity().claims_cells())
                .then(|| (entity, entity_def::mover_shape(registry, form)))
        })
        .collect()
}
