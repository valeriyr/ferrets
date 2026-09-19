//! Die order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_physics::body;

use ferrets_content::dying::DyingDef;

use super::orders::{Processing, Refusal};
use crate::{
    components::{
        dying::{DiedComponent, DyingComponent, Passing},
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    map::{Map, OccupancyClass},
    order::Order,
    spawn::{self, Fallen},
};

/// Whether `entity` may start a Die: always.
pub fn can_start(_world: &World, _entity: Entity, _order: &Order) -> Result<(), Refusal> {
    Ok(())
}

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
) -> Processing {
    Processing::state(OrderState::InProcessing)
}

/// Whether a Die can stand through a soft cancel: always — dying cannot be
/// called off.
pub fn survives_soft_cancel() -> bool {
    true
}

/// Advance a Die order by one tick.
///
/// Counts down the dying timer. When it expires, [`DyingComponent`] is replaced
/// with [`DiedComponent`], the source the entity was raised over (if any) is
/// put back, what the death hands on is left behind, and the order
/// finishes.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let passing = {
        let mut entity_mut = world.entity_mut(entity);
        let mut dying = entity_mut
            .get_mut::<DyingComponent>()
            .expect("a Die order requires DyingComponent on the entity");

        if dying.ticks_remaining > 0 {
            dying.ticks_remaining -= 1;
            return Processing::state(OrderState::InProcessing);
        }

        // Read before the component goes: what the death settled about what it
        // leaves is held nowhere else.
        let passing = dying.passing;
        entity_mut.remove::<DyingComponent>();
        entity_mut.insert(DiedComponent);
        passing
    };

    // The decay has run out, so a body stops being one the moment it ends: it
    // must not go on filling a slot the cap counts while the death it is in
    // the middle of hands on what it leaves — a body that rots into another
    // would be refused the room it just freed — and a cast must not raise
    // what is already gone.
    let id = entity_def::simulation_id(world, entity);
    world.resource_mut::<EntityIndex>().remove_remains(id);

    free_footprint(world, entity);
    // The berth the entity held through its dying phase — one it died in with
    // no free cell to step back onto — goes back to the job.
    spawn::unseat(world, entity);
    spawn::uncover_source(world, entity);
    leave_bequests(world, entity, passing);
    Processing::state(OrderState::Finished)
}

/// Frees the footprint the entity has held through its dying phase, so the
/// remains it leaves behind (or anyone else) can take the cells.
///
/// An entity off the grid — hidden, or attached to a job — holds nothing.
fn free_footprint(world: &mut World, entity: Entity) {
    if !entity_def::stands_on_grid(world, entity) {
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

/// Hands on what the entity's type leaves for the death it died.
///
/// Each bequest names a type and how many of it: anything tagged as remains is
/// set down as a body — ownerless, lying where it fell, remembering who it was
/// — and anything else stands up as an ordinary entity of whoever owned the
/// deceased. A weapon that leaves nothing of what it kills denies the bodies
/// and nothing else: what bursts out of a dying thing is that thing's own doing.
///
/// Called after the footprint is freed, so the first cells the bequests are
/// offered are the ones the deceased was standing on.
fn leave_bequests(world: &mut World, entity: Entity, passing: Passing) {
    let (kind, slain) = match passing {
        Passing::Death { kind, slain } => (kind, slain),
        Passing::Removal => return,
    };
    let left: Vec<(String, u32)> = entity_def::of(world, entity)
        .dying
        .as_ref()
        .map(DyingDef::leaves)
        .unwrap_or_default()
        .iter()
        .filter(|bequest| bequest.left_by(kind))
        .map(|bequest| (bequest.entity_type().to_string(), bequest.count()))
        .collect();
    if left.is_empty() {
        return;
    }
    // A death off the map hands nothing on: there is no ground under what went
    // down inside a carrier or inside the site it was spent on, and the cell it
    // last stood on is wherever it happened to step aboard.
    if world.entity(entity).contains::<HiddenComponent>() {
        return;
    }

    let position = entity_def::position(world, entity);
    // What is left rests on the lattice: a continuous mover dies wherever
    // pushing left it, and what it leaves takes the cell its body stood on. On
    // the cell model the two coincide.
    let around = body::anchor(position);
    let size = entity_def::of(world, entity)
        .location
        .expect("a standing entity has a location")
        .size();
    let fallen = Fallen {
        id: entity_def::simulation_id(world, entity),
        entity_type: entity_def::type_id(world, entity),
        owner: entity_def::owner(world, entity),
    };

    for (type_name, count) in left {
        spawn::spawn_bequest(world, &type_name, count, around, size, fallen, slain);
    }
}
