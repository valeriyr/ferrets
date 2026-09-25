//! Per-tick command dispatch: translates the buffered input of
//! [`InputFrames`](crate::input::InputFrames) into order-queue mutations.
//!
//! Commands only ever affect entities owned by the issuing player; selection is
//! the single exception — any visible entity can be selected.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use super::{build, cast, morph, orders};
use crate::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{
        build::UnderConstructionComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner,
        rally::{RallyPointComponent, RallyTarget},
        stance::StanceComponent,
        tags::TagsComponent,
        train::{TrainComponent, TrainQueueComponent},
    },
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    entity_def,
    entity_index::EntityIndex,
    events::{SpawnCause, SpendCause},
    input::InputFrames,
    order::{AttackTarget, Order},
    player_research::PlayerResearch,
    requirements,
    resources::{self, PlayerResources},
    selection::Selection,
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
    supply, visibility,
};
use ferrets_content::{
    registry::ContentRegistry,
    research::ResearchId,
    skills::{SkillCaster, SkillId},
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
            if visibility::interactable_to(world, player, *id).is_some() {
                apply_selection(world, player, vec![*id], *mode);
            }
        }
        PlayerCommand::SelectByIds { ids, mode } => {
            let selected: Vec<SimulationId> = ids
                .iter()
                .copied()
                .filter(|&id| visibility::interactable_to(world, player, id).is_some())
                .collect();
            apply_selection(world, player, selected, *mode);
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
                .filter(|&id| visibility::interactable_to(world, player, id).is_some())
                .collect();
            // Recalling an empty (or fully-wiped) group is a no-op: it must not
            // clear the current selection.
            if candidates.is_empty() {
                return;
            }
            apply_selection(world, player, candidates, *mode);
        }
        PlayerCommand::Move { target, flush } => {
            let commanded = commanded_selection(world, player);
            issue(
                world,
                commanded,
                Order::Move {
                    target: *target,
                    size: CellSize::ONE,
                    range: 0,
                },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Attack { target, flush } => {
            // An explicit attack is honored as given — including force-attacking an
            // own or allied unit. Only the smart send-to-entity order below refuses
            // to attack a non-hostile target; whether friendly-fire damage lands is
            // a game-rules concern, not the command executor's.
            //
            // A named target is never ordered to attack itself; a cell excludes nobody.
            // A named target must be in sight to be named at all — fog
            // refuses the order the way it hides the sprite.
            if let Some(id) = target.entity()
                && visibility::interactable_to(world, player, id).is_none()
            {
                return;
            }
            let commanded = match target.entity() {
                Some(id) => commanded_selection_excluding(world, player, id),
                None => commanded_selection(world, player),
            };
            issue(
                world,
                commanded,
                Order::Attack {
                    target: *target,
                    leash: None,
                },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::AttackMove { target, flush } => {
            let commanded = commanded_selection(world, player);
            issue(
                world,
                commanded,
                Order::AttackMove { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Patrol { target, flush } => {
            let commanded = commanded_selection(world, player);
            issue(
                world,
                commanded,
                Order::Patrol { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Guard { target, flush } => {
            if visibility::interactable_to(world, player, *target).is_none() {
                return;
            }
            let commanded = commanded_selection_excluding(world, player, *target);
            issue(
                world,
                commanded,
                Order::Guard { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::SetStance { stance } => {
            for entity in commanded_selection(world, player) {
                if let Some(mut current) = world.entity_mut(entity).get_mut::<StanceComponent>() {
                    current.0 = *stance;
                }
            }
        }
        PlayerCommand::SendToEntity { target, flush } => {
            if visibility::interactable_to(world, player, *target).is_none() {
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
                && visibility::interactable_to(world, player, *id).is_none()
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
            issue(
                world,
                vec![entity],
                Order::Build {
                    type_name: type_name.clone(),
                    position: *position,
                },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::CancelTrain { trainer, slot } => {
            cancel_train(world, player, *trainer, *slot)
        }
        PlayerCommand::CancelResearch {
            researcher,
            research,
        } => cancel_research(world, player, *researcher, *research),
        PlayerCommand::CancelBuild { site } => cancel_build(world, player, *site),
        PlayerCommand::CancelMorph { entity } => cancel_morph(world, player, *entity),
        PlayerCommand::Repair { target, flush } => {
            if visibility::interactable_to(world, player, *target).is_none() {
                return;
            }
            // A mixed selection sends the ones that can mend this target.
            let commanded = commanded_selection(world, player);
            issue(
                world,
                commanded,
                Order::Repair { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Follow { target, flush } => {
            // Following what the fog hides would be a tracking beacon.
            if visibility::interactable_to(world, player, *target).is_none() {
                return;
            }
            let commanded = commanded_selection_excluding(world, player, *target);
            issue(
                world,
                commanded,
                Order::Follow { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Board { target, flush } => {
            // A mixed selection sends the ones the target takes aboard.
            if visibility::interactable_to(world, player, *target).is_none() {
                return;
            }
            let commanded = commanded_selection_excluding(world, player, *target);
            issue(
                world,
                commanded,
                Order::Board { target: *target },
                CancelPolicy::from_bool(*flush),
            );
        }
        PlayerCommand::Load {
            transport,
            target,
            flush,
        } => {
            let Some(entity) = find_owned_interactable(world, player, *transport) else {
                return;
            };
            if visibility::interactable_to(world, player, *target).is_none() {
                return;
            }
            issue(
                world,
                vec![entity],
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
            issue(
                world,
                vec![entity],
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
        PlayerCommand::Morph { type_name, flush } => {
            // Requirements gate only the command, as they do for training;
            // costs and ground are settled when the order actually starts.
            let commanded: Vec<Entity> = commanded_selection(world, player)
                .into_iter()
                .filter(|&entity| morph::requirements_met(world, player, entity, type_name))
                .collect();
            issue(
                world,
                commanded,
                Order::Morph {
                    type_name: type_name.clone(),
                },
                CancelPolicy::from_bool(*flush),
            );
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
            spawn::spawn_entity(
                world,
                type_name,
                corner,
                Some(player),
                SpawnCause::Sandbox,
                FieldReach::Initial,
            );
        }
    }
}

/// Resolves a send-to-entity intent for one unit, by priority: harvest from a
/// source or deliver to a storage, join the crew raising a site, mend, board,
/// attack a hostile, follow. The first order the unit may start wins.
pub(super) fn resolve_send_to_entity(
    world: &World,
    entity: Entity,
    target_id: SimulationId,
) -> Option<Order> {
    let target = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)?;

    let mut candidates = vec![Order::Harvest { target: target_id }];
    // A site still going up is a call for help, read before a delivery: a
    // half-built storage is not a drop-off yet however much its type says it
    // accepts the load — and Harvest refuses it for that reason.
    candidates.extend(assist_construction(world, entity, target));
    candidates.push(Order::Repair { target: target_id });
    // Boarding comes after repair, so a worker sent to a damaged transport
    // patches it up instead of climbing in.
    candidates.push(Order::Board { target: target_id });
    // An explicit Attack command honours any target; the smart click attacks
    // only a hostile one, and a weapon that cannot reach it falls through to
    // following, which is the honest reading of the click.
    if owner::are_hostile(
        world.resource::<GameSession>(),
        entity_def::owner(world, entity),
        entity_def::owner(world, target),
    ) && world.entity(target).contains::<HealthComponent>()
    {
        candidates.push(Order::Attack {
            target: AttackTarget::Entity(target_id),
            leash: None,
        });
    }
    candidates.push(Order::Follow { target: target_id });

    candidates
        .into_iter()
        .find(|order| orders::can_start(world, entity, order).is_ok())
}

/// The Build order that puts `entity` to work on the unfinished site `target`, or
/// `None` if `target` is not an own site.
///
/// The order names the site's own type and cell, which is what
/// [`game_loop::build`](super::build) matches an existing site on — so the builder
/// takes up the work already under way rather than trying to place a second one.
/// Only the owner's own sites qualify.
fn assist_construction(world: &World, entity: Entity, target: Entity) -> Option<Order> {
    let target_ref = world.entity(target);
    if !target_ref.contains::<UnderConstructionComponent>() {
        return None;
    }
    let same_owner = matches!(
        (entity_def::owner(world, entity), entity_def::owner(world, target)),
        (Some(builder), Some(site)) if builder == site
    );
    if !same_owner {
        return None;
    }
    let type_name = target_ref
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .type_name();

    Some(Order::Build {
        type_name: type_name.to_string(),
        position: entity_def::position(world, target),
    })
}

/// Validates and executes a train command: pays the price up front and enqueues
/// the unit; a cancel aimed at the slot pays it back.
fn train_entity(world: &mut World, player: PlayerId, trainer: SimulationId, type_name: &str) {
    let Some(entity) = find_owned_interactable(world, player, trainer) else {
        return;
    };
    if orders::can_start(world, entity, &Order::Train).is_err() {
        return;
    }
    if !entity_def::of(world, entity)
        .trainer
        .as_ref()
        .is_some_and(|t| t.can_train(type_name))
    {
        return;
    }

    let Some((price, supply_ok, requirements_ok)) = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .filter(|def| def.train_time.is_some())
        .map(|def| {
            (
                def.price.clone(),
                supply::allows(world, player, def),
                requirements::met(world, player, Some(entity), &def.requires),
            )
        })
    else {
        return;
    };
    // Supply is reserved here, where the price is paid: the queue entry
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
        .can_afford(player, &price)
    {
        return;
    }
    resources::charge(world, player, price, SpendCause::Training { trainer });

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

/// Validates and executes a cancel-training command: drops the entry at `slot`
/// of the trainer's production queue and refunds its price.
///
/// Slot 0 is the unit in progress; dropping it restarts the entry behind it
/// from no progress. Nothing happens for a trainer that is not the player's,
/// for one with no production queue, or for a slot past the end of it. The
/// Train order finishes on its own once the queue it works is empty.
fn cancel_train(world: &mut World, player: PlayerId, trainer: SimulationId, slot: u8) {
    let Some(entity) = find_owned_interactable(world, player, trainer) else {
        return;
    };
    let type_name = {
        let mut entity_mut = world.entity_mut(entity);
        let Some(mut queue) = entity_mut.get_mut::<TrainQueueComponent>() else {
            return;
        };
        match queue.0.remove(slot as usize) {
            Some(type_name) => type_name,
            None => return,
        }
    };

    // The entry in progress carried the progress counter: what follows it
    // starts fresh rather than inheriting ticks paid toward another type.
    if slot == 0
        && let Some(mut train_component) = world.entity_mut(entity).get_mut::<TrainComponent>()
    {
        train_component.progress = 0;
    }

    let price = world
        .resource::<ContentRegistry>()
        .entity(&type_name)
        .expect("a queued entry names a type the registry minted it from")
        .price
        .clone();
    resources::refund(world, player, price, SpendCause::Training { trainer });
}

/// Validates and executes a research command: pays the price up front and pushes
/// the order; a cancel aimed at the topic pays it back.
fn start_research(
    world: &mut World,
    player: PlayerId,
    researcher: SimulationId,
    research: ResearchId,
) {
    let Some(entity) = find_owned_interactable(world, player, researcher) else {
        return;
    };
    if orders::can_start(world, entity, &Order::Research { research }).is_err() {
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
    let Some((price, requires)) = world
        .resource::<ContentRegistry>()
        .research_def(research)
        .map(|def| (def.price.clone(), def.requires.clone()))
    else {
        return;
    };
    if !requirements::met(world, player, Some(entity), &requires) {
        return;
    }
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &price)
    {
        return;
    }
    resources::charge(world, player, price, SpendCause::Research { research });

    let mut entity_mut = world.entity_mut(entity);
    let mut queue = entity_mut
        .get_mut::<OrderQueueComponent>()
        .expect("simulation entities always have an order queue");
    queue.push(Order::Research { research }, None);
}

/// Validates and executes a cancel-research command: calls off `research` on
/// the researcher working it and refunds its price.
///
/// Nothing happens for a researcher that is not the player's, or for a topic
/// that entity has neither queued nor under way. The entry is marked forced,
/// so a topic still waiting in the queue drops as surely as the one in hand.
fn cancel_research(
    world: &mut World,
    player: PlayerId,
    researcher: SimulationId,
    research: ResearchId,
) {
    let Some(entity) = find_owned_interactable(world, player, researcher) else {
        return;
    };
    let mut entity_mut = world.entity_mut(entity);
    let Some(mut queue) = entity_mut.get_mut::<OrderQueueComponent>() else {
        return;
    };
    // Only the named topic is called off: the rest of the queue, production
    // and other topics alike, is none of this command's business.
    let Some(entry) = queue.0.iter_mut().find(
        |entry| matches!(entry.order, Order::Research { research: queued } if queued == research),
    ) else {
        return;
    };
    // The entry stays in the queue until the order loop flushes it, a system
    // set later, so a second command naming the same topic in one frame finds
    // it again. The mandatory mark is what the refund is paid for: an entry
    // already carrying one has been paid for once. An advisory mark is one a
    // research entry refuses, so it has paid for nothing and is raised.
    match entry.cancel {
        Some(CancelPolicy::Force) => return,
        Some(CancelPolicy::Soft) | None => entry.cancel = Some(CancelPolicy::Force),
    }

    let price = world
        .resource::<ContentRegistry>()
        .research_def(research)
        .expect("a queued topic carries a registry-minted id")
        .price
        .clone();
    resources::refund(world, player, price, SpendCause::Research { research });
}

/// Validates and executes a cancel-build command: tears down the unfinished
/// `site` and refunds what it cost. Whoever is working it finds the site gone
/// on its next tick and steps off.
///
/// Nothing happens for a site that is not the player's, is finished, or is
/// already gone.
fn cancel_build(world: &mut World, player: PlayerId, site: SimulationId) {
    let Some(building) = find_owned_interactable(world, player, site) else {
        return;
    };
    if !world
        .entity(building)
        .contains::<UnderConstructionComponent>()
    {
        return;
    }
    let price = entity_def::of(world, building).price.clone();
    build::tear_down_site(world, building);
    resources::refund(world, player, price, SpendCause::Construction { site });
}

/// Validates and executes a cancel-morph command: calls off the change of form
/// `entity` is under.
///
/// The change answers on its own terms, as it does for any other cancel: a
/// refundable one gives the costs back and returns the entity to what it was,
/// a forfeit one keeps them, and a committed one holds until its window
/// closes. Nothing happens for an entity that is not the player's or is not
/// changing at all.
fn cancel_morph(world: &mut World, player: PlayerId, entity: SimulationId) {
    let Some(entity) = find_owned_interactable(world, player, entity) else {
        return;
    };
    let mut entity_mut = world.entity_mut(entity);
    let Some(mut queue) = entity_mut.get_mut::<OrderQueueComponent>() else {
        return;
    };
    let Some(front) = queue.front_mut() else {
        return;
    };
    // Only the change itself is called off: whatever else the entity has
    // queued is none of this command's business.
    match front.order {
        Order::Morph { .. } => front.cancel = Some(CancelPolicy::Soft),
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::AttackMove { .. }
        | Order::Patrol { .. }
        | Order::Guard { .. }
        | Order::Follow { .. }
        | Order::Board { .. }
        | Order::Load { .. }
        | Order::Unload { .. }
        | Order::Harvest { .. }
        | Order::Build { .. }
        | Order::Repair { .. }
        | Order::Train
        | Order::Research { .. }
        | Order::Cast { .. }
        | Order::Die => {}
    }
}

/// Whether any of the player's entities is already working on or queued for
/// the given research.
fn research_in_flight(world: &World, player: PlayerId, research: ResearchId) -> bool {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_def::owner(world, entity) != Some(player) {
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
            visibility::interactable_to(world, player, id).is_some()
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
        .filter(|&&(_, entity)| entity_def::owner(world, entity) == Some(player))
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
                && entity_def::owner(world, entity) == Some(player)
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

/// Resolves `id` if it is interactable and owned by `player`.
fn find_owned_interactable(world: &World, player: PlayerId, id: SimulationId) -> Option<Entity> {
    let entity = world.resource::<EntityIndex>().interactable(world, id)?;
    (entity_def::owner(world, entity) == Some(player)).then_some(entity)
}

/// Pushes `order` on each of `entities` that may start it now (see
/// [`orders::can_start`]); the rest are refused without a trace, so a mixed
/// selection simply sends the ones that can.
fn issue(world: &mut World, entities: Vec<Entity>, order: Order, flush: Option<CancelPolicy>) {
    for entity in entities {
        if orders::can_start(world, entity, &order).is_ok() {
            push_order(world, entity, order.clone(), flush);
        }
    }
}

fn push_order(world: &mut World, entity: Entity, order: Order, flush: Option<CancelPolicy>) {
    if let Some(mut queue) = world.entity_mut(entity).get_mut::<OrderQueueComponent>() {
        queue.push(order, flush);
    }
}

/// Casts a skill for `player`: the skill must exist, match the caster kind it
/// is asked of, and its requirements must be met.
///
/// A player cast is settled here and now. An entity cast becomes an order, so
/// what it is aimed at, what it costs and how far it must walk are all settled
/// when the order runs — but a caster whose skill is still cooling down, or
/// that cannot start the order at all, is left at whatever it was doing rather
/// than dropping it for a cast that will not happen.
fn use_skill(
    world: &mut World,
    player: PlayerId,
    skill: SkillId,
    caster: SkillCasterRef,
    target: Option<SkillTarget>,
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
    // skill unlocks with its owner's research, and locks again with it. An
    // annex entry is asked of the caster, so a player cast can hold none.
    let casting = match caster {
        SkillCasterRef::Entity(id) => find_owned_interactable(world, player, id),
        SkillCasterRef::Player => None,
    };
    if !requirements::met(world, player, casting, &def.requires) {
        return;
    }
    match (caster, &def.caster) {
        (SkillCasterRef::Player, SkillCaster::Player { price, effect }) => {
            cast::by_player(world, player, skill, def.cooldown, price, *effect);
        }
        (SkillCasterRef::Entity(_), SkillCaster::Entity { .. }) => {
            let Some(entity) = casting else {
                return;
            };
            // Casting is something the caster does, so it goes through the
            // queue like any other doing: what it was at is canceled, the
            // order walks it into reach if the skill has one, and the cast
            // answers to the same gating, refusals and cancellation as the
            // rest. Judged before the queue is touched, though — a cast the
            // caster could not start, or one still cooling down, must not
            // cancel what it was doing for nothing.
            let order = Order::Cast { skill, target };
            if cast::can_start(world, entity, &order).is_err() || !cast::ready(world, entity, skill)
            {
                return;
            }
            issue(world, vec![entity], order, Some(CancelPolicy::Soft));
        }
        (SkillCasterRef::Player, SkillCaster::Entity { .. })
        | (SkillCasterRef::Entity(_), SkillCaster::Player { .. }) => {}
    }
}
