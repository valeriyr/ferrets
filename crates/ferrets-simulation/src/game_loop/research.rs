//! Research order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::orders::{self, Processing, Refusal};
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderState},
        research::ResearchComponent,
    },
    entity_def,
    events::SpendCause,
    game_loop::stats,
    order::Order,
    player_research::{self, PlayerResearch},
    resources,
};
use ferrets_content::registry::ContentRegistry;

/// Whether `entity` may start this Research: its type hosts the topic and it
/// stands raised. A disabled researcher is admitted and waits.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let Order::Research { research } = order else {
        unreachable!("can_start called with a non-Research order");
    };
    if !entity_def::of(world, entity)
        .researcher
        .as_ref()
        .is_some_and(|r| r.can_research(*research))
    {
        return Err(Refusal::Incapable);
    }
    orders::requires_raised(world, entity)
}

/// Called once when a Research order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Order::Research { research } = order else {
        unreachable!("prepare called with a non-Research order");
    };
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(ResearchComponent {
        research: *research,
        progress: 0,
    });
    OrderState::InProcessing
}

/// Called for every Research entry that has a cancel policy.
///
/// A soft cancel is refused — the work continues. A force cancel refunds the
/// full cost to the owner, discards the progress, and finishes.
pub fn cancel_processing(
    entity: Entity,
    order: &Order,
    policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    let Order::Research { research } = order else {
        unreachable!("cancel_processing called with a non-Research order");
    };

    match policy {
        CancelPolicy::Soft => OrderState::InProcessing,
        CancelPolicy::Force => {
            let owner = entity_def::owner(world, entity);
            if let Some(player) = owner {
                let cost = world
                    .resource::<ContentRegistry>()
                    .research_def(*research)
                    .expect("research orders carry a registry-minted id")
                    .cost
                    .clone();
                resources::refund(
                    world,
                    player,
                    cost,
                    SpendCause::Research {
                        research: *research,
                    },
                );
            }

            world.entity_mut(entity).remove::<ResearchComponent>();
            OrderState::Finished
        }
    }
}

/// Advance a Research order by one tick.
///
/// Each tick the progress grows; when the research time is reached, the
/// research is marked completed for the owning player, its buff (when it
/// carries one) is applied, and the order finishes.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut research_component) = world.entity_mut(entity).take::<ResearchComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let research = research_component.research;

    let Some(player) = entity_def::owner(world, entity) else {
        return Processing::state(OrderState::Finished);
    };
    let (cost, research_time, buff) = {
        let def = world
            .resource::<ContentRegistry>()
            .research_def(research)
            .expect("research orders carry a registry-minted id");
        (def.cost.clone(), def.research_time, def.buff)
    };

    // Completed in the meantime — nothing left to work toward, so the payment
    // comes back. The executor refuses a duplicate while an order is in
    // flight, so this only covers a completion that landed outside the order
    // path (a scenario script granting the research) after this order was
    // already paid for.
    if world
        .resource::<PlayerResearch>()
        .is_completed(player, research)
    {
        resources::refund(world, player, cost, SpendCause::Research { research });
        return Processing::state(OrderState::Finished);
    }

    research_component.progress += 1;
    if research_component.progress < research_time {
        world.entity_mut(entity).insert(research_component);
        return Processing::state(OrderState::InProcessing);
    }

    let researcher = Some(entity_def::simulation_id(world, entity));
    player_research::complete(world, player, research, researcher);
    if let Some(buff) = buff {
        stats::apply_player_buff(world, player, buff);
    }
    Processing::state(OrderState::Finished)
}
