//! Train order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::rally;
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        train::{TrainComponent, TrainQueueComponent},
    },
    entity_def,
    events::{SpawnCause, SpendCause},
    map::Map,
    order::Order,
    resources, spawn,
};
use ferrets_content::registry::ContentRegistry;

/// Called once when a Train order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot train or has nothing queued.
pub fn prepare(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    if entity_def::of(world, entity).trainer.is_none() {
        return OrderState::Finished;
    }
    let entity_ref = world.entity(entity);
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
                    resources::refund(
                        world,
                        player,
                        cost,
                        SpendCause::Training {
                            trainer: entity_def::simulation_id(world, entity),
                        },
                    );
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

    let allowed = entity_def::of(world, entity)
        .trainer
        .as_ref()
        .is_some_and(|t| t.can_train(&type_name));

    let (train_time, unit_location_def) = {
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
        let (position, size) = entity_def::footprint(world, entity);
        let origin = CellPos::from(position);

        let placement =
            world
                .resource::<Map>()
                .find_placement_near(origin, size, &unit_location_def);

        // No free cell around the trainer — hold the finished unit and retry.
        if let Some(cell) = placement {
            let owner = world
                .entity(entity)
                .get::<OwnerComponent>()
                .map(|o| o.player());
            let trainer = entity_def::simulation_id(world, entity);
            if let Some((unit, _)) = spawn::spawn_entity(
                world,
                &type_name,
                FixedUVec2::from(cell),
                owner,
                SpawnCause::Trained { trainer },
            ) {
                rally::send(world, entity, unit);
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
