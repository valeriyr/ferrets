//! Per-tick command dispatch: translates the buffered input of
//! [`InputFrames`](crate::input::InputFrames) into order-queue mutations.
//!
//! Commands only ever affect entities owned by the issuing player; selection is
//! the single exception — any visible entity can be selected.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    command::PlayerCommand,
    components::{
        attack::AttackStaticData,
        build::BuilderStaticData,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::{self, OwnerComponent},
        resource::{
            ResourceCarrierComponent, ResourceCarrierStaticData, ResourceSourceComponent,
            ResourceSourceStaticData, ResourceStorageStaticData,
        },
        train::{TrainQueueComponent, TrainStaticData},
    },
    content::registry::ContentRegistry,
    entity_index::EntityIndex,
    input::InputFrames,
    order::Order,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, player_slot::PlayerId},
    simulation_id::SimulationId,
    spawn,
};

/// Processes the frame for `current_tick` once every player the tick requires
/// (see [`GameSession::required_players`]) has contributed.
///
/// Returns `true` if the frame was ready and processed, `false` if the tick should block.
pub fn tick(world: &mut World, current_tick: u32) -> bool {
    let required = world
        .resource::<GameSession>()
        .required_players(current_tick);
    let Some(ready) = world
        .resource::<InputFrames>()
        .ready_commands(current_tick, &required)
    else {
        return false;
    };

    let commands: Vec<(PlayerId, Vec<PlayerCommand>)> = ready
        .into_iter()
        .map(|(player, commands)| (player, commands.to_vec()))
        .collect();

    for (player, player_commands) in &commands {
        for command in player_commands {
            execute(world, *player, command);
        }
    }

    true
}

fn execute(world: &mut World, player: PlayerId, command: &PlayerCommand) {
    match command {
        PlayerCommand::SelectById { id } => {
            if world
                .resource::<EntityIndex>()
                .interactable(world, *id)
                .is_some()
            {
                world.resource_mut::<Selection>().set(player, vec![*id]);
            }
        }
        PlayerCommand::SelectByRect { rect } => {
            let selected: Vec<SimulationId> = world
                .resource::<EntityIndex>()
                .alive_entries()
                .into_iter()
                .filter(|&(id, entity)| {
                    world
                        .resource::<EntityIndex>()
                        .interactable(world, id)
                        .is_some()
                        && world
                            .entity(entity)
                            .get::<LocationComponent>()
                            .is_some_and(|loc| rect.contains(loc.position))
                })
                .map(|(id, _)| id)
                .collect();

            world.resource_mut::<Selection>().set(player, selected);
        }
        PlayerCommand::Move { target, flush } => {
            for entity in commanded_selection(world, player) {
                push_order(
                    world,
                    entity,
                    Order::Move {
                        target: *target,
                        range: 0,
                    },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Attack { target, flush } => {
            // An explicit attack is honored as given — including force-attacking an
            // own or allied unit. Only the smart send-to-entity order below refuses
            // to attack a non-hostile target; whether friendly-fire damage lands is
            // a game-rules concern, not the command executor's.
            for entity in commanded_selection_excluding(world, player, *target) {
                push_order(
                    world,
                    entity,
                    Order::Attack { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::SendToEntity { target, flush } => {
            if world
                .resource::<EntityIndex>()
                .interactable(world, *target)
                .is_none()
            {
                return;
            }
            for entity in commanded_selection_excluding(world, player, *target) {
                if let Some(order) = resolve_send_to_entity(world, entity, *target) {
                    push_order(world, entity, order, CancelPolicy::from_bool(*flush));
                }
            }
        }
        PlayerCommand::TrainEntity { trainer, type_name } => {
            train_entity(world, player, *trainer, type_name);
        }
        PlayerCommand::BuildEntity {
            builder,
            type_name,
            position,
            flush,
        } => {
            let Some(entity) = find_owned_interactable(world, player, *builder) else {
                return;
            };
            if !world
                .entity(entity)
                .get::<BuilderStaticData>()
                .is_some_and(|b| b.can_build(type_name))
            {
                return;
            }
            let constructible = world
                .resource::<ContentRegistry>()
                .entity(type_name)
                .is_some_and(|def| def.build_time.is_some());
            if !constructible {
                return;
            }
            push_order(
                world,
                entity,
                Order::Build {
                    type_name: type_name.clone(),
                    position: *position,
                },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Stop => {
            for entity in commanded_selection(world, player) {
                if let Some(mut queue) = world.entity_mut(entity).get_mut::<OrderQueueComponent>() {
                    queue.cancel_all(CancelPolicy::Soft);
                }
            }
        }
        PlayerCommand::Spawn {
            type_name,
            position,
        } => {
            // Sandbox spawn: no-op if the type is unknown or the cell is blocked.
            spawn::spawn_entity(world, type_name, *position, Some(player));
        }
    }
}

/// Resolves a send-to-entity intent for one unit, by priority: harvest from a
/// source, deliver carried resources to an own storage, attack a hostile, follow.
fn resolve_send_to_entity(world: &World, entity: Entity, target_id: SimulationId) -> Option<Order> {
    let target = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)?;
    let entity_ref = world.entity(entity);
    let target_ref = world.entity(target);

    let carries_source_kind = entity_ref
        .get::<ResourceCarrierStaticData>()
        .zip(target_ref.get::<ResourceSourceStaticData>())
        .is_some_and(|(carrier, source)| carrier.can_carry(source.kind()));
    if carries_source_kind && target_ref.contains::<ResourceSourceComponent>() {
        return Some(Order::Harvest { target: target_id });
    }

    let carried = entity_ref
        .get::<ResourceCarrierComponent>()
        .map_or(0, |carrier| carrier.amount);
    // Only a storage the carrier itself owns is a drop-off — not an ally's or a
    // neutral one (matches the storage the delivery actually resolves to, see
    // `resolve_storage`). Being non-hostile is not enough now that allies exist.
    let own_storage = matches!(
        (
            entity_ref.get::<OwnerComponent>(),
            target_ref.get::<OwnerComponent>(),
        ),
        (Some(carrier), Some(storage)) if carrier.player() == storage.player()
    );
    let accepts_delivery = carried > 0
        && own_storage
        && entity_ref
            .get::<ResourceCarrierComponent>()
            .and_then(|carrier| carrier.kind.as_deref())
            .zip(target_ref.get::<ResourceStorageStaticData>())
            .is_some_and(|(kind, storage)| storage.accepts(kind));
    if accepts_delivery {
        return Some(Order::Harvest { target: target_id });
    }

    let hostile = owner::are_hostile(
        world.resource::<GameSession>(),
        entity_ref.get::<OwnerComponent>(),
        target_ref.get::<OwnerComponent>(),
    );
    if hostile
        && entity_ref.contains::<AttackStaticData>()
        && target_ref.contains::<HealthComponent>()
    {
        return Some(Order::Attack { target: target_id });
    }

    Some(Order::Follow { target: target_id })
}

/// Validates and executes a train command: pays the cost up front and enqueues
/// the unit; production refunds on a force cancel.
fn train_entity(world: &mut World, player: PlayerId, trainer: SimulationId, type_name: &str) {
    let Some(entity) = find_owned_interactable(world, player, trainer) else {
        return;
    };
    if !world
        .entity(entity)
        .get::<TrainStaticData>()
        .is_some_and(|t| t.can_train(type_name))
    {
        return;
    }

    let Some(cost) = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .filter(|def| def.train_time.is_some())
        .map(|def| def.cost.clone())
    else {
        return;
    };
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &cost)
    {
        return;
    }
    world
        .resource_mut::<PlayerResources>()
        .subtract(player, &cost);

    world
        .entity_mut(entity)
        .get_mut::<TrainQueueComponent>()
        .expect("trainers always have a train queue")
        .0
        .push_back(type_name.to_string());

    // One Train order works through the whole queue; only push when none is queued.
    let mut entity_mut = world.entity_mut(entity);
    let mut queue = entity_mut
        .get_mut::<OrderQueueComponent>()
        .expect("simulation entities always have an order queue");
    let already_training = queue.0.iter().any(|e| matches!(e.order, Order::Train));
    if !already_training {
        queue.push(Order::Train, None);
    }
}

/// The player's currently selected entities that the player may command.
fn commanded_selection(world: &mut World, player: PlayerId) -> Vec<Entity> {
    world
        .resource::<Selection>()
        .get(player)
        .to_owned()
        .into_iter()
        .filter_map(|id| find_owned_interactable(world, player, id))
        .collect()
}

/// Like [`commanded_selection`], skipping `excluded` (e.g. a command's own target).
fn commanded_selection_excluding(
    world: &mut World,
    player: PlayerId,
    excluded: SimulationId,
) -> Vec<Entity> {
    world
        .resource::<Selection>()
        .get(player)
        .to_owned()
        .into_iter()
        .filter(|&id| id != excluded)
        .filter_map(|id| find_owned_interactable(world, player, id))
        .collect()
}

/// Resolves `id` if it is interactable and owned by `player`.
fn find_owned_interactable(world: &World, player: PlayerId, id: SimulationId) -> Option<Entity> {
    let entity = world.resource::<EntityIndex>().interactable(world, id)?;
    world
        .entity(entity)
        .get::<OwnerComponent>()
        .filter(|o| o.player() == player)
        .map(|_| entity)
}

fn push_order(world: &mut World, entity: Entity, order: Order, flush: Option<CancelPolicy>) {
    if let Some(mut queue) = world.entity_mut(entity).get_mut::<OrderQueueComponent>() {
        queue.push(order, flush);
    }
}
