//! Train order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::nav_pos::NavPos;

use crate::{
    components::{
        location::{LocationComponent, LocationStaticData},
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        train::{TrainComponent, TrainQueueComponent, TrainStaticData},
    },
    content::registry::ContentRegistry,
    game_loop::executor,
    map::Map,
    order::Order,
    resources::PlayerResources,
    spawn,
};

/// Called once when a Train order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot train or has nothing queued.
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    let entity_ref = world.entity(entity);
    if !entity_ref.contains::<TrainStaticData>() {
        return OrderState::Finished;
    }
    if entity_ref
        .get::<TrainQueueComponent>()
        .is_none_or(|queue| queue.0.is_empty())
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(TrainComponent::default());
    OrderState::InProcessing
}

/// Called for every Train entry that has a cancel policy.
///
/// A soft cancel is refused — production continues. A force cancel refunds every
/// queued entry to the owner, clears the queue, and finishes.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    match policy {
        CancelPolicy::Soft => OrderState::InProcessing,
        CancelPolicy::Force => {
            let owner = world
                .entity(entity)
                .get::<OwnerComponent>()
                .map(|o| o.player());
            let queued: Vec<String> = world
                .entity_mut(entity)
                .get_mut::<TrainQueueComponent>()
                .map(|mut q| q.0.drain(..).collect())
                .unwrap_or_default();

            if let Some(player) = owner {
                for type_name in &queued {
                    let cost = world
                        .resource::<ContentRegistry>()
                        .entity(type_name)
                        .map(|def| def.cost.clone())
                        .unwrap_or_default();
                    world
                        .resource_mut::<PlayerResources>()
                        .refund(player, &cost);
                }
            }

            world.entity_mut(entity).remove::<TrainComponent>();
            OrderState::Finished
        }
    }
}

/// Advance a Train order by one tick.
///
/// Each tick the front queue entry progresses. When its train time is reached, the
/// unit is spawned on the nearest free cell around the trainer's footprint and the
/// entry is dequeued; with no free cell the unit waits, retrying every tick. The
/// order finishes when the queue is empty.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    let Some(mut train_component) = world.entity_mut(entity).take::<TrainComponent>() else {
        return OrderState::Finished;
    };

    let Some(type_name) = world
        .entity(entity)
        .get::<TrainQueueComponent>()
        .and_then(|queue| queue.0.front().cloned())
    else {
        return OrderState::Finished;
    };

    let allowed = world
        .entity(entity)
        .get::<TrainStaticData>()
        .is_some_and(|t| t.can_train(&type_name));

    let (train_time, unit_location_data) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = if allowed {
            registry.entity(&type_name)
        } else {
            None
        };
        let Some(type_def) = type_def else {
            // An entry this entity may not train, or whose type vanished from
            // the registry — drop it.
            world
                .entity_mut(entity)
                .get_mut::<TrainQueueComponent>()
                .unwrap()
                .0
                .pop_front();
            world.entity_mut(entity).insert(train_component);
            return OrderState::InProcessing;
        };
        (
            type_def.train_time.expect("queued type must be trainable"),
            type_def
                .location
                .expect("validated content defines a location"),
        )
    };

    if train_component.progress < train_time {
        train_component.progress += 1;
    }

    if train_component.progress >= train_time {
        let origin = NavPos::from(
            world
                .entity(entity)
                .get::<LocationComponent>()
                .unwrap()
                .position,
        );
        let size = world
            .entity(entity)
            .get::<LocationStaticData>()
            .unwrap()
            .size();

        let placement =
            world
                .resource::<Map>()
                .find_placement_near(origin, size, &unit_location_data);

        // No free cell around the trainer — hold the finished unit and retry.
        if let Some(cell) = placement {
            let owner = world
                .entity(entity)
                .get::<OwnerComponent>()
                .map(|o| o.player());
            if let Some((unit, _)) =
                spawn::spawn_entity(world, &type_name, FixedUVec2::from(cell), owner)
            {
                send_to_rally(entity, unit, world);
            }

            let mut entity_mut = world.entity_mut(entity);
            let mut queue = entity_mut.get_mut::<TrainQueueComponent>().unwrap();
            queue.0.pop_front();
            train_component.progress = 0;

            if queue.0.is_empty() {
                return OrderState::Finished;
            }
        }
    }

    world.entity_mut(entity).insert(train_component);
    OrderState::InProcessing
}

/// Sends a freshly spawned unit toward the trainer's rally point, if one is set.
///
/// A position rallies as a plain move; an entity resolves like a send-to-entity
/// intent from the unit's own perspective (e.g. a worker harvests a source, a
/// soldier attacks a hostile). A rally target gone by spawn time issues nothing.
fn send_to_rally(trainer: Entity, unit: Entity, world: &mut World) {
    let Some(target) = world
        .entity(trainer)
        .get::<RallyPointComponent>()
        .and_then(|rally| rally.0)
    else {
        return;
    };

    let order = match target {
        RallyTarget::Position(position) => Some(Order::Move {
            target: position,
            range: 0,
        }),
        RallyTarget::Entity(id) => executor::resolve_send_to_entity(world, unit, id),
    };
    if let Some(order) = order
        && let Some(mut queue) = world.entity_mut(unit).get_mut::<OrderQueueComponent>()
    {
        queue.push(order, None);
    }
}
