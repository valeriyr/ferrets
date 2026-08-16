//! Die order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_physics::body;

use crate::{
    components::{
        dying::{DiedComponent, DyingComponent},
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    map::{Map, OccupancyClass},
    order::Order,
    spawn,
};

/// Called once when a Die order becomes the front `New` entry.
///
/// Asserts the [`DyingComponent`] driver is present and returns `InProcessing`;
/// the dying phase needs no further setup.
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    debug_assert!(
        world.entity(entity).contains::<DyingComponent>(),
        "a Die order requires DyingComponent on the entity"
    );
    OrderState::InProcessing
}

/// Called for every Die entry that has a cancel policy.
///
/// Dying cannot be cancelled; the entry always stays in the queue.
pub fn cancel_processing(
    _entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    _world: &mut World,
) -> OrderState {
    OrderState::InProcessing
}

/// Advance a Die order by one tick.
///
/// Counts down the dying timer. When it expires, [`DyingComponent`] is replaced
/// with [`DiedComponent`], the configured corpse (if any) is left behind, and
/// the order finishes.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    {
        let mut entity_mut = world.entity_mut(entity);
        let mut dying = entity_mut
            .get_mut::<DyingComponent>()
            .expect("a Die order requires DyingComponent on the entity");

        if dying.ticks_remaining > 0 {
            dying.ticks_remaining -= 1;
            return OrderState::InProcessing;
        }

        entity_mut.remove::<DyingComponent>();
        entity_mut.insert(DiedComponent);
    }

    free_footprint(entity, world);
    leave_corpse(entity, world);
    OrderState::Finished
}

/// Frees the footprint the entity has held through its dying phase, so the
/// remains it leaves behind (or anyone else) can take the cells.
///
/// Hidden entities are off the map and hold nothing.
fn free_footprint(entity: Entity, world: &mut World) {
    if world.entity(entity).contains::<HiddenComponent>() {
        return;
    }

    let location = *world.entity(entity).get::<LocationComponent>().unwrap();
    let def = entity_def::of(world, entity);
    let location_def = def.location.unwrap();
    let class = OccupancyClass::of(def);
    world
        .resource_mut::<Map>()
        .displace_entity(&location, &location_def, class);
}

/// Leaves the entity's configured corpse at its position, if any.
///
/// The corpse is born dying: its own dying phase acts as the decay timer, and a
/// corpse type with a corpse of its own forms the next decay stage. It claims
/// its footprint on the navigation grid per its occupation mask, so remains can
/// block movement (rubble) or lie passable (a corpse layer movers ignore).
/// When the footprint is blocked — someone took the cell during the death — no
/// remains are left.
fn leave_corpse(entity: Entity, world: &mut World) {
    let Some(corpse_type) = entity_def::of(world, entity)
        .dying
        .as_ref()
        .and_then(|dying| dying.corpse_type().map(String::from))
    else {
        return;
    };
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    // Remains rest on the lattice: a continuous mover dies wherever pushing
    // left it, and the corpse takes the cell the body visually stood on. On
    // the cell model the two coincide.
    let cell = FixedUVec2::from(body::anchor(position));

    spawn::spawn_corpse_entity(world, &corpse_type, cell);
}
