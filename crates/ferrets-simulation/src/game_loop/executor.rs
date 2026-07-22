//! Per-tick command dispatch: translates the buffered input of
//! [`InputFrames`](crate::input::InputFrames) into order-queue mutations.
//!
//! Commands only ever affect entities owned by the issuing player; selection is
//! the single exception — any visible entity can be selected.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_urect::FixedURect;

use crate::{
    command::{PlayerCommand, SelectMode},
    components::{
        attack::AttackStaticData,
        build::{BuilderStaticData, UnderConstructionComponent},
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::{self, OwnerComponent},
        rally::{RallyPointComponent, RallyTarget},
        resource::{
            ResourceCarrierComponent, ResourceCarrierStaticData, ResourceSourceComponent,
            ResourceSourceStaticData, ResourceStorageStaticData,
        },
        stance::StanceComponent,
        tags::{self, TagsComponent},
        train::{TrainQueueComponent, TrainStaticData},
    },
    content::registry::ContentRegistry,
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
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
        PlayerCommand::SelectById { id, mode } => {
            if world
                .resource::<EntityIndex>()
                .interactable(world, *id)
                .is_some()
            {
                apply_selection(world, player, vec![*id], *mode);
            }
        }
        PlayerCommand::SelectByRect { rect, mode } => {
            let selected = resolve_box_selection(world, player, rect);
            apply_selection(world, player, selected, *mode);
        }
        PlayerCommand::SelectByType { class, rect, mode } => {
            let selected = resolve_type_selection(world, player, class, rect);
            apply_selection(world, player, selected, *mode);
        }
        PlayerCommand::AssignGroup { group } => {
            let group = *group as usize;
            if group < CONTROL_GROUP_COUNT {
                let ids = world.resource::<Selection>().get(player).to_vec();
                world
                    .resource_mut::<ControlGroups>()
                    .assign(player, group, ids);
            }
        }
        PlayerCommand::AppendGroup { group } => {
            let group = *group as usize;
            if group < CONTROL_GROUP_COUNT {
                let ids = world.resource::<Selection>().get(player).to_vec();
                world
                    .resource_mut::<ControlGroups>()
                    .append(player, group, &ids);
            }
        }
        PlayerCommand::RecallGroup { group, mode } => {
            let group = *group as usize;
            if group >= CONTROL_GROUP_COUNT {
                return;
            }
            // A group prunes destroyed ids on despawn, but a dying entity may
            // still be listed — recall only what is currently interactable, as
            // the other selection commands do.
            let candidates: Vec<SimulationId> = world
                .resource::<ControlGroups>()
                .get(player, group)
                .to_vec()
                .into_iter()
                .filter(|&id| {
                    world
                        .resource::<EntityIndex>()
                        .interactable(world, id)
                        .is_some()
                })
                .collect();
            // Recalling an empty (or fully-wiped) group is a no-op: it must not
            // clear the current selection.
            if candidates.is_empty() {
                return;
            }
            apply_selection(world, player, candidates, *mode);
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
                    Order::Attack {
                        target: *target,
                        leash: None,
                    },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::AttackMove { target, flush } => {
            for entity in commanded_selection(world, player) {
                push_order(
                    world,
                    entity,
                    Order::AttackMove { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Patrol { target, flush } => {
            for entity in commanded_selection(world, player) {
                push_order(
                    world,
                    entity,
                    Order::Patrol { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Guard { target, flush } => {
            let Some(ward) = world.resource::<EntityIndex>().interactable(world, *target) else {
                return;
            };
            // Guarding is for own, allied, and neutral wards — a hostile ward
            // would immediately become the guard's own scan target.
            if let Some(owner) = world.entity(ward).get::<OwnerComponent>()
                && !world
                    .resource::<GameSession>()
                    .are_allied(player, owner.player())
            {
                return;
            }
            for entity in commanded_selection_excluding(world, player, *target) {
                push_order(
                    world,
                    entity,
                    Order::Guard { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::SetStance { stance } => {
            for entity in commanded_selection(world, player) {
                if let Some(mut current) = world.entity_mut(entity).get_mut::<StanceComponent>() {
                    current.0 = *stance;
                }
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
        PlayerCommand::SetRallyPoint { entity, target } => {
            let Some(entity) = find_owned_interactable(world, player, *entity) else {
                return;
            };
            // An entity target must exist when the rally point is set, matching
            // the send-to-entity rule; it may be gone again by the time a unit
            // spawns, which spawn-time resolution handles.
            if let Some(RallyTarget::Entity(id)) = target
                && world
                    .resource::<EntityIndex>()
                    .interactable(world, *id)
                    .is_none()
            {
                return;
            }
            if let Some(mut rally) = world.entity_mut(entity).get_mut::<RallyPointComponent>() {
                rally.0 = *target;
            }
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
pub(super) fn resolve_send_to_entity(
    world: &World,
    entity: Entity,
    target_id: SimulationId,
) -> Option<Order> {
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
        return Some(Order::Attack {
            target: target_id,
            leash: None,
        });
    }

    Some(Order::Follow { target: target_id })
}

/// Validates and executes a train command: pays the cost up front and enqueues
/// the unit; production refunds on a force cancel.
fn train_entity(world: &mut World, player: PlayerId, trainer: SimulationId, type_name: &str) {
    let Some(entity) = find_owned_interactable(world, player, trainer) else {
        return;
    };
    // A building still being constructed cannot produce yet.
    if world
        .entity(entity)
        .contains::<UnderConstructionComponent>()
    {
        return;
    }
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

/// Resolves a box selection: interactable entities inside `rect` that are not
/// buildings, narrowed to the issuing player's own units when the box caught any.
///
/// Buildings are excluded from a rect selection (they can still be selected
/// individually). When the box holds no own units it falls back to a single
/// other-owner entity so an enemy or neutral can still be boxed to inspect it.
fn resolve_box_selection(world: &World, player: PlayerId, rect: &FixedURect) -> Vec<SimulationId> {
    let index = world.resource::<EntityIndex>();
    let in_rect: Vec<(SimulationId, Entity)> = index
        .alive_entries()
        .into_iter()
        .filter(|&(id, entity)| {
            index.interactable(world, id).is_some()
                && world
                    .entity(entity)
                    .get::<LocationComponent>()
                    .is_some_and(|loc| rect.contains(loc.position))
                && !world
                    .entity(entity)
                    .get::<TagsComponent>()
                    .is_some_and(|component| component.contains(tags::BUILDING))
        })
        .collect();

    let own: Vec<SimulationId> = in_rect
        .iter()
        .filter(|&&(_, entity)| {
            world
                .entity(entity)
                .get::<OwnerComponent>()
                .is_some_and(|owner| owner.player() == player)
        })
        .map(|&(id, _)| id)
        .collect();

    if own.is_empty() {
        in_rect
            .into_iter()
            .next()
            .map(|(id, _)| id)
            .into_iter()
            .collect()
    } else {
        own
    }
}

/// Resolves a select-by-class: interactable entities inside `rect` whose
/// registered selection class equals `class`, restricted to the issuing player's
/// own entities (grouping by class covers your own units, not the enemy's).
fn resolve_type_selection(
    world: &World,
    player: PlayerId,
    class: &str,
    rect: &FixedURect,
) -> Vec<SimulationId> {
    let index = world.resource::<EntityIndex>();
    let registry = world.resource::<ContentRegistry>();
    index
        .alive_entries()
        .into_iter()
        .filter(|&(id, entity)| {
            index.interactable(world, id).is_some()
                && world
                    .entity(entity)
                    .get::<OwnerComponent>()
                    .is_some_and(|owner| owner.player() == player)
                && world
                    .entity(entity)
                    .get::<LocationComponent>()
                    .is_some_and(|loc| rect.contains(loc.position))
                && world
                    .entity(entity)
                    .get::<EntityInfoComponent>()
                    .and_then(|info| registry.entity(info.type_name()))
                    .is_some_and(|def| def.selection_class() == class)
        })
        .map(|(id, _)| id)
        .collect()
}

/// Combines `candidates` into `player`'s selection according to `mode`.
fn apply_selection(
    world: &mut World,
    player: PlayerId,
    candidates: Vec<SimulationId>,
    mode: SelectMode,
) {
    let mut selection = world.resource_mut::<Selection>();
    match mode {
        SelectMode::Replace => selection.set(player, candidates),
        SelectMode::Add => selection.add(player, &candidates),
        SelectMode::Toggle => selection.toggle(player, &candidates),
        SelectMode::Remove => selection.subtract(player, &candidates),
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
