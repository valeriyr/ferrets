//! Research order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    components::{
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        research::ResearchComponent,
    },
    entity_def,
    game_loop::stats,
    order::Order,
    player_research::PlayerResearch,
    resources::PlayerResources,
};
use ferrets_content::registry::ContentRegistry;

/// Called once when a Research order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot host this research.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Order::Research { research } = order else {
        unreachable!("prepare called with a non-Research order");
    };

    if !entity_def::of(world, entity)
        .researcher
        .as_ref()
        .is_some_and(|r| r.can_research(*research))
    {
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
            let owner = world
                .entity(entity)
                .get::<OwnerComponent>()
                .map(|o| o.player());
            if let Some(player) = owner {
                let cost = world
                    .resource::<ContentRegistry>()
                    .research_def(*research)
                    .expect("research orders carry a registry-minted id")
                    .cost
                    .clone();
                world
                    .resource_mut::<PlayerResources>()
                    .refund(player, &cost);
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
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    let Some(mut research_component) = world.entity_mut(entity).take::<ResearchComponent>() else {
        return OrderState::Finished;
    };
    let research = research_component.research;

    let Some(player) = world
        .entity(entity)
        .get::<OwnerComponent>()
        .map(|o| o.player())
    else {
        return OrderState::Finished;
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
        world
            .resource_mut::<PlayerResources>()
            .refund(player, &cost);
        return OrderState::Finished;
    }

    research_component.progress += 1;
    if research_component.progress < research_time {
        world.entity_mut(entity).insert(research_component);
        return OrderState::InProcessing;
    }

    world
        .resource_mut::<PlayerResearch>()
        .complete(player, research);
    if let Some(buff) = buff {
        stats::apply_player_buff(world, player, buff);
    }
    OrderState::Finished
}
