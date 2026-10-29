//! Train order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{landing::Landing, registry::ContentRegistry};
use ferrets_geometry::cell_rect::CellRect;
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::orders::{self, Processing, Refusal};
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderState},
        train::{TrainComponent, TrainQueueComponent},
    },
    entity_def,
    landing::{self, Bound, ComesFrom, Lander, Landings},
    make_way::{self, Asked},
    order::Order,
    rally,
    spawn::{self, FieldReach},
};

/// Whether `entity` may start a Train: its type trains and it has risen.
/// A disabled trainer is admitted and waits; an orphaned annex is not.
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    if entity_def::of(world, entity).trainer.is_none() {
        return Err(Refusal::Incapable);
    }
    orders::admits_queued_work(world, entity)
}

/// Called once when a Train order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`] — or nothing
/// is queued.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
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
/// A soft cancel is refused and production carries on. A force cancel empties
/// the queue and finishes, and what its entries cost stays spent.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> Processing {
    match policy {
        CancelPolicy::Soft => Processing::state(OrderState::InProcessing),
        CancelPolicy::Force => {
            let mut entity_mut = world.entity_mut(entity);
            if let Some(mut queue) = entity_mut.get_mut::<TrainQueueComponent>() {
                queue.0.clear();
            }
            entity_mut.remove::<TrainComponent>();
            Processing::state(OrderState::Finished)
        }
    }
}

/// Whether a Train can stand through a soft cancel: always — only a force
/// cancel takes production away.
pub fn survives_soft_cancel() -> bool {
    true
}

/// Advance a Train order by one tick.
///
/// Each tick the front queue entry progresses. When its train time is reached, the
/// unit is spawned on the nearest free cell around the trainer's footprint and the
/// entry is dequeued; with no free cell the unit waits, retrying every tick. The
/// order finishes when the queue is empty.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut train_component) = world.entity_mut(entity).take::<TrainComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let Some(type_name) = world
        .entity(entity)
        .get::<TrainQueueComponent>()
        .and_then(|queue| queue.0.front().cloned())
    else {
        return Processing::state(OrderState::Finished);
    };

    let allowed = entity_def::of(world, entity)
        .trainer
        .as_ref()
        .is_some_and(|t| t.can_train(&type_name));

    let train_time = {
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
            return Processing::state(OrderState::InProcessing);
        };
        type_def.train_time.expect("queued type must be trainable")
    };

    if train_component.progress < train_time {
        train_component.progress += 1;
    }

    if train_component.progress >= train_time {
        let trainer_rect = entity_def::footprint_rect(world, entity);
        let owner = entity_def::owner(world, entity)
            .expect("a train queue is filled by its owner's command");
        let mut landings = Landings::new(owner, Landing::Anywhere, &[], &[], &[]);
        let placement = landing::land(
            world,
            &mut landings,
            unit_lander(world, entity, &type_name, trainer_rect),
        );
        let spawned = match placement {
            // No free cell around the trainer: own and allied movers that
            // clear and alone keep the unit off one are asked off it if they
            // yield, and the finished unit is held and retried.
            None => {
                match make_way::clear_landing(
                    world,
                    Some(owner),
                    |world| unit_lander(world, entity, &type_name, trainer_rect),
                    &[],
                    &[],
                ) {
                    Asked::Leaves | Asked::Boxed => {}
                }
                None
            }
            // The unit leaves the queue only once it stands: a spawn the cell
            // refuses after all holds it the same way.
            Some(cell) => {
                let trainer = entity_def::simulation_id(world, entity);
                spawn::spawn_trained(
                    world,
                    &type_name,
                    FixedUVec2::from(cell),
                    owner,
                    FieldReach::Initial,
                    trainer,
                )
            }
        };
        if let Some((unit, _)) = spawned {
            rally::mark_rally_due(world, entity, unit);

            let mut entity_mut = world.entity_mut(entity);
            let mut queue = entity_mut.get_mut::<TrainQueueComponent>().unwrap();
            queue.0.pop_front();
            train_component.progress = 0;

            if queue.0.is_empty() {
                return Processing::state(OrderState::Finished);
            }
        }
    }

    world.entity_mut(entity).insert(train_component);
    Processing::state(OrderState::InProcessing)
}

/// How a unit of `type_name` trained by `entity` comes out: beside
/// `trainer_rect`, the trainer's footprint, toward it.
///
/// Panics for a type the registry does not hold.
fn unit_lander<'w>(
    world: &'w World,
    entity: Entity,
    type_name: &str,
    trainer_rect: CellRect,
) -> Lander<'w> {
    Lander {
        def: world
            .resource::<ContentRegistry>()
            .entity(type_name)
            .expect("the type was resolved a moment ago"),
        owner: entity_def::owner(world, entity),
        from: ComesFrom::of(world, entity),
        bound: Bound::Radius,
        around: trainer_rect,
        toward: trainer_rect,
    }
}
