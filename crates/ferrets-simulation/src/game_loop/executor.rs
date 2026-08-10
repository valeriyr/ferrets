//! Per-tick command dispatch: translates the buffered input of
//! [`InputFrames`](crate::input::InputFrames) into order-queue mutations.
//!
//! Commands only ever affect entities owned by the issuing player; selection is
//! the single exception — any visible entity can be selected.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use super::{board, repair, stats};
use crate::{
    command::{PlayerCommand, SelectMode, SkillCasterRef},
    components::{
        build::UnderConstructionComponent,
        energy::EnergyComponent,
        entity_buffs::BuffsComponent,
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::{self, OwnerComponent},
        rally::{RallyPointComponent, RallyTarget},
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::StanceComponent,
        tags::TagsComponent,
        train::TrainQueueComponent,
    },
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    entity_def,
    entity_index::EntityIndex,
    input::InputFrames,
    order::{AttackTarget, Order},
    player_buffs::PlayerBuffs,
    player_research::PlayerResearch,
    player_skills::PlayerSkills,
    requirements,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, player_slot::PlayerId},
    simulation_id::SimulationId,
    spawn, supply,
};
use ferrets_content::{
    costs::Cost,
    entity_stats::EntityStatId,
    projectile::Aim,
    registry::ContentRegistry,
    research::ResearchId,
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillId,
    },
    tags,
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
                        size: CellSize::ONE,
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
            //
            // A named target is never ordered to attack itself; a cell excludes nobody.
            let commanded = match target.entity() {
                Some(id) => commanded_selection_excluding(world, player, id),
                None => commanded_selection(world, player),
            };
            let aimed_at_ground = target.entity().is_none();
            for entity in commanded {
                // Only a weapon that sends its shots to a cell can be aimed at one.
                if aimed_at_ground && !aims_at_cells(world, entity) {
                    continue;
                }
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
        PlayerCommand::StartResearch {
            researcher,
            research,
        } => {
            start_research(world, player, *researcher, *research);
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
            if !entity_def::of(world, entity)
                .builder
                .as_ref()
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
        PlayerCommand::Repair { target, flush } => {
            // Entities that cannot mend this target drop the order in `prepare`, so
            // a mixed selection simply sends the ones that can.
            for entity in commanded_selection(world, player) {
                push_order(
                    world,
                    entity,
                    Order::Repair { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Follow { target, flush } => {
            if world
                .resource::<EntityIndex>()
                .interactable(world, *target)
                .is_none()
            {
                return;
            }
            for entity in commanded_selection_excluding(world, player, *target) {
                push_order(
                    world,
                    entity,
                    Order::Follow { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Board { target, flush } => {
            // Entities the target will not take aboard drop the order in
            // `prepare`, so a mixed selection simply sends the ones that fit.
            if world
                .resource::<EntityIndex>()
                .interactable(world, *target)
                .is_none()
            {
                return;
            }
            for entity in commanded_selection_excluding(world, player, *target) {
                push_order(
                    world,
                    entity,
                    Order::Board { target: *target },
                    CancelPolicy::from_bool(*flush),
                );
            }
        }
        PlayerCommand::Load {
            transport,
            target,
            flush,
        } => {
            let Some(entity) = find_owned_interactable(world, player, *transport) else {
                return;
            };
            push_order(
                world,
                entity,
                Order::Load { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Unload {
            transport,
            at,
            flush,
        } => {
            // Only the holder's owner opens the hold, whoever the passengers
            // belong to.
            let Some(entity) = find_owned_interactable(world, player, *transport) else {
                return;
            };
            push_order(
                world,
                entity,
                Order::Unload { at: *at },
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
        PlayerCommand::UseSkill {
            skill,
            caster,
            target,
        } => {
            use_skill(world, player, *skill, *caster, *target);
        }
        PlayerCommand::Spawn {
            type_name,
            position,
        } => {
            // Sandbox spawn: no-op if the type is unknown or the cell is
            // blocked. The position comes off the wire, so it is floored to
            // the cell origin the spawn contract requires rather than
            // trusted to be one.
            let corner = FixedUVec2::from(CellPos::from(*position));
            spawn::spawn_entity(world, type_name, corner, Some(player));
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

    let carries_source_kind = entity_def::of(world, entity)
        .resource_carrier
        .as_ref()
        .zip(entity_def::of(world, target).resource_source.as_ref())
        .is_some_and(|(carrier, source)| carrier.can_carry(source.kind()));
    if carries_source_kind && target_ref.contains::<ResourceSourceComponent>() {
        return Some(Order::Harvest { target: target_id });
    }

    // A site still going up is a call for help: a unit that builds its type joins the
    // crew raising it. Read before the delivery below, because a half-built storage
    // is not a drop-off yet however much its type says it accepts the load.
    if target_ref.contains::<UnderConstructionComponent>()
        && let Some(order) = assist_construction(world, entity, target)
    {
        return Some(order);
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
            .zip(entity_def::of(world, target).resource_storage.as_ref())
            .is_some_and(|(kind, storage)| storage.accepts(kind));
    if accepts_delivery {
        return Some(Order::Harvest { target: target_id });
    }

    // A damaged friendly the unit can mend, after the harvest readings so a loaded
    // carrier still delivers to a storage that happens to be hurt.
    if repair::would_repair(world, entity, target) {
        return Some(Order::Repair { target: target_id });
    }

    // A transporter with room takes the unit aboard — after repair, so a worker
    // sent to a damaged transport patches it up instead of climbing in.
    if board::would_board(world, entity, target) {
        return Some(Order::Board { target: target_id });
    }

    let hostile = owner::are_hostile(
        world.resource::<GameSession>(),
        entity_ref.get::<OwnerComponent>(),
        target_ref.get::<OwnerComponent>(),
    );
    if hostile
        && entity_def::of(world, entity).can_attack()
        && target_ref.contains::<HealthComponent>()
    {
        return Some(Order::Attack {
            target: AttackTarget::Entity(target_id),
            leash: None,
        });
    }

    Some(Order::Follow { target: target_id })
}

/// The Build order that puts `entity` to work on the unfinished site `target`, or
/// `None` if it is not a site this one can join.
///
/// The order names the site's own type and cell, which is what
/// [`game_loop::build`](super::build) matches an existing site on — so the builder
/// takes up the work already under way rather than trying to place a second one.
/// Only the owner's own sites qualify, because that is the whole of what can be
/// joined.
fn assist_construction(world: &World, entity: Entity, target: Entity) -> Option<Order> {
    let target_ref = world.entity(target);

    let same_owner = matches!(
        (
            world.entity(entity).get::<OwnerComponent>(),
            target_ref.get::<OwnerComponent>(),
        ),
        (Some(builder), Some(site)) if builder.player() == site.player()
    );
    if !same_owner {
        return None;
    }

    let type_name = target_ref
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .type_name();
    if !entity_def::of(world, entity)
        .builder
        .as_ref()
        .is_some_and(|builder| builder.can_build(type_name))
    {
        return None;
    }

    Some(Order::Build {
        type_name: type_name.to_string(),
        position: target_ref
            .get::<LocationComponent>()
            .expect("a placed site has a location")
            .position,
    })
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
    if !entity_def::of(world, entity)
        .trainer
        .as_ref()
        .is_some_and(|t| t.can_train(type_name))
    {
        return;
    }

    let Some((cost, supply_ok, requirements_ok)) = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .filter(|def| def.train_time.is_some())
        .map(|def| {
            (
                def.cost.clone(),
                supply::allows(world, player, def),
                requirements::met(world, player, &def.requires),
            )
        })
    else {
        return;
    };
    // Supply is reserved here, where the resource cost is paid: the queue entry
    // holds it from this moment and hands it to the unit it becomes.
    if !supply_ok {
        return;
    }
    // Requirements gate only the command: an entry already queued keeps
    // training even when its requirement falls.
    if !requirements_ok {
        return;
    }
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

/// Validates and executes a research command: pays the cost up front and pushes
/// the order; the work refunds on a force cancel.
fn start_research(
    world: &mut World,
    player: PlayerId,
    researcher: SimulationId,
    research: ResearchId,
) {
    let Some(entity) = find_owned_interactable(world, player, researcher) else {
        return;
    };
    // A building still being constructed cannot research yet.
    if world
        .entity(entity)
        .contains::<UnderConstructionComponent>()
    {
        return;
    }
    if !entity_def::of(world, entity)
        .researcher
        .as_ref()
        .is_some_and(|r| r.can_research(research))
    {
        return;
    }
    if world
        .resource::<PlayerResearch>()
        .is_completed(player, research)
    {
        return;
    }
    // One research per topic per player, everywhere: derived from the order
    // queues themselves, so a researcher that dies never leaves the topic
    // locked.
    if research_in_flight(world, player, research) {
        return;
    }

    // Resolved defensively: the id arrives over the wire, and an id this
    // registry never minted is a peer to distrust, not a panic.
    let Some((cost, requires)) = world
        .resource::<ContentRegistry>()
        .research_def(research)
        .map(|def| (def.cost.clone(), def.requires.clone()))
    else {
        return;
    };
    if !requirements::met(world, player, &requires) {
        return;
    }
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &cost)
    {
        return;
    }
    world
        .resource_mut::<PlayerResources>()
        .subtract(player, &cost);

    let mut entity_mut = world.entity_mut(entity);
    let mut queue = entity_mut
        .get_mut::<OrderQueueComponent>()
        .expect("simulation entities always have an order queue");
    queue.push(Order::Research { research }, None);
}

/// Whether any of the player's entities is already working on or queued for
/// the given research.
fn research_in_flight(world: &World, player: PlayerId, research: ResearchId) -> bool {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_ref
            .get::<OwnerComponent>()
            .is_none_or(|owner| owner.player() != player)
        {
            continue;
        }
        let Some(queue) = entity_ref.get::<OrderQueueComponent>() else {
            continue;
        };
        if queue
            .0
            .iter()
            .any(|entry| matches!(&entry.order, Order::Research { research: r } if *r == research))
        {
            return true;
        }
    }
    false
}

/// Resolves a box selection: interactable entities inside `rect` that are not
/// buildings, narrowed to the issuing player's own units when the box caught any.
///
/// Buildings are excluded from a rect selection (they can still be selected
/// individually). When the box holds no own units it falls back to a single
/// other-owner entity so an enemy or neutral can still be boxed to inspect it.
/// The rectangle tests footprint centers, so boxing matches what an entity
/// is drawn as even when a continuous mover rests between cell origins.
fn resolve_box_selection(world: &World, player: PlayerId, rect: &FixedURect) -> Vec<SimulationId> {
    let index = world.resource::<EntityIndex>();
    let in_rect: Vec<(SimulationId, Entity)> = index
        .alive_entries()
        .into_iter()
        .filter(|&(id, entity)| {
            index.interactable(world, id).is_some()
                && world.entity(entity).contains::<LocationComponent>()
                && rect.contains(entity_def::footprint_center(world, entity))
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
                && world.entity(entity).contains::<LocationComponent>()
                && rect.contains(entity_def::footprint_center(world, entity))
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

/// Whether the entity's weapon sends its shots to a cell rather than following a
/// target — the only kind that can be aimed at bare ground.
fn aims_at_cells(world: &World, entity: Entity) -> bool {
    entity_def::of(world, entity)
        .projectile
        .is_some_and(|projectile| {
            world
                .resource::<ContentRegistry>()
                .projectile_def(projectile)
                .aim()
                == Aim::Position
        })
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

/// Validates and executes a cast: the skill must exist, match its caster
/// kind, be off cooldown for that caster, be affordable (every cost payable),
/// and the target must be valid for the skill.
fn use_skill(
    world: &mut World,
    player: PlayerId,
    skill: SkillId,
    caster: SkillCasterRef,
    target_id: Option<SimulationId>,
) {
    // Resolved defensively: the id arrives over the wire, and an id this
    // registry never minted is a peer to distrust, not a panic. The same goes
    // for a caster ref that does not match the skill's cast arm.
    let Some(def) = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .cloned()
    else {
        return;
    };
    // Requirements answer to the issuing player whoever casts: an entity's
    // skill unlocks with its owner's research, and locks again with it.
    if !requirements::met(world, player, &def.requires) {
        return;
    }
    match (caster, def.caster) {
        (SkillCasterRef::Player, SkillCaster::Player { cost, effect }) => {
            use_skill_as_player(world, player, skill, def.cooldown, &cost, effect);
        }
        (
            SkillCasterRef::Entity(caster_id),
            SkillCaster::Entity {
                costs,
                target,
                effect,
            },
        ) => {
            use_skill_as_entity(
                world,
                player,
                skill,
                def.cooldown,
                &costs,
                target,
                effect,
                caster_id,
                target_id,
            );
        }
        (SkillCasterRef::Player, SkillCaster::Entity { .. })
        | (SkillCasterRef::Entity(_), SkillCaster::Player { .. }) => {}
    }
}

/// The player-cast path: cooldown per player, resource cost, effect on the
/// casting player.
fn use_skill_as_player(
    world: &mut World,
    player: PlayerId,
    skill: SkillId,
    cooldown: u32,
    cost: &Cost,
    effect: PlayerCastEffect,
) {
    if !world.resource::<PlayerSkills>().ready(player, skill) {
        return;
    }
    if !world.resource::<PlayerResources>().can_afford(player, cost) {
        return;
    }
    world
        .resource_mut::<PlayerResources>()
        .subtract(player, cost);

    match effect {
        PlayerCastEffect::ApplyBuff(buff) => stats::apply_player_buff(world, player, buff),
        PlayerCastEffect::RemoveBuff(buff) => {
            world.resource_mut::<PlayerBuffs>().remove(player, buff);
        }
    }

    world
        .resource_mut::<PlayerSkills>()
        .start_cooldown(player, skill, cooldown);
}

/// The entity-cast path: the caster must be an owned entity whose type
/// declares the skill; cooldown per entity, pool costs draw from the caster,
/// the effect lands on the resolved target entity.
#[allow(clippy::too_many_arguments)]
fn use_skill_as_entity(
    world: &mut World,
    player: PlayerId,
    skill: SkillId,
    cooldown: u32,
    costs: &[EntityCastCost],
    cast_target: EntityCastTarget,
    effect: EntityCastEffect,
    caster_id: SimulationId,
    target_id: Option<SimulationId>,
) {
    let Some(caster) = find_owned_interactable(world, player, caster_id) else {
        return;
    };
    // The caster must have the skill, and it must be off cooldown.
    if !world
        .entity(caster)
        .get::<SkillsComponent>()
        .is_some_and(|skills| skills.ready(skill))
    {
        return;
    }

    // Resolve and validate the target.
    let target = match cast_target {
        EntityCastTarget::Caster => caster,
        EntityCastTarget::Ally | EntityCastTarget::Enemy => {
            let Some(target_id) = target_id else {
                return;
            };
            let Some(target) = world
                .resource::<EntityIndex>()
                .interactable(world, target_id)
            else {
                return;
            };
            let session = world.resource::<GameSession>();
            let caster_ref = world.entity(caster);
            let target_ref = world.entity(target);
            let caster_owner = caster_ref.get::<OwnerComponent>();
            let target_owner = target_ref.get::<OwnerComponent>();
            let valid = match cast_target {
                EntityCastTarget::Ally => matches!(
                    (caster_owner, target_owner),
                    (Some(caster), Some(target)) if session.are_allied(caster.player(), target.player())
                ),
                EntityCastTarget::Enemy => owner::are_hostile(session, caster_owner, target_owner),
                EntityCastTarget::Caster => unreachable!("handled above"),
            };
            if !valid {
                return;
            }
            target
        }
    };

    // Fold the costs into one total per pool, so a check covers every arm that
    // draws from that pool.
    let mut resources = Cost::new();
    let mut energy_cost = FixedU64::ZERO;
    let mut health_cost = FixedU64::ZERO;
    for cost in costs {
        match cost {
            EntityCastCost::Resources(cost) => {
                for (kind, amount) in cost {
                    *resources.entry(kind.clone()).or_default() += amount;
                }
            }
            EntityCastCost::Energy(amount) => energy_cost += *amount,
            EntityCastCost::Health(amount) => health_cost += *amount,
        }
    }

    // Every cost must be payable before any is paid, so a cast never
    // half-charges.
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &resources)
    {
        return;
    }
    let caster_ref = world.entity(caster);
    if energy_cost > FixedU64::ZERO
        && caster_ref
            .get::<EnergyComponent>()
            .is_none_or(|energy| energy.current() < energy_cost)
    {
        return;
    }
    // Strictly more health than the cost: a cast that could not be survived is
    // refused.
    if health_cost > FixedU64::ZERO
        && caster_ref
            .get::<HealthComponent>()
            .is_none_or(|health| health.current() <= health_cost)
    {
        return;
    }

    world
        .resource_mut::<PlayerResources>()
        .subtract(player, &resources);
    let mut caster_mut = world.entity_mut(caster);
    if energy_cost > FixedU64::ZERO
        && let Some(mut energy) = caster_mut.get_mut::<EnergyComponent>()
    {
        energy.spend(energy_cost);
    }
    if health_cost > FixedU64::ZERO
        && let Some(mut health) = caster_mut.get_mut::<HealthComponent>()
    {
        health.apply_damage(health_cost);
    }

    apply_skill_effect(world, caster, target, effect);

    if let Some(mut skills) = world.entity_mut(caster).get_mut::<SkillsComponent>() {
        skills.start_cooldown(skill, cooldown);
    }
}

/// Applies a resolved skill effect to `target`.
fn apply_skill_effect(world: &mut World, caster: Entity, target: Entity, effect: EntityCastEffect) {
    match effect {
        EntityCastEffect::ApplyBuff(id) => super::stats::apply_entity_buff(world, target, id),
        EntityCastEffect::RemoveBuff(id) => {
            if let Some(mut buffs) = world.entity_mut(target).get_mut::<BuffsComponent>() {
                buffs.remove(id);
            }
        }
        EntityCastEffect::Damage(amount) => {
            let caster_id = entity_def::simulation_id(world, caster);
            let tick = world.resource::<GameSession>().tick();
            let mut died = false;
            if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
                // Skill damage bypasses armor, like an ability rather than a weapon.
                health.apply_damage(amount);
                health.record_hit(caster_id, tick);
                died = health.is_dead();
            }
            if died {
                spawn::destroy_entity(world, target);
            }
        }
        EntityCastEffect::Heal(amount) => {
            let max = world
                .entity(target)
                .get::<StatsComponent>()
                .and_then(|stats| stats.effective(EntityStatId::MAX_HEALTH))
                .unwrap_or(FixedU64::ZERO);
            if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
                health.heal(amount, max);
            }
        }
    }
}
