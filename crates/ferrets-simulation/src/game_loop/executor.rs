//! Per-tick command dispatch: translates the buffered input of
//! [`InputFrames`](crate::input::InputFrames) into order-queue mutations.
//!
//! Commands only ever affect entities owned by the issuing player; selection is
//! the single exception — any visible entity can be selected.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use super::{morph, orders, stats};
use crate::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{
        build::UnderConstructionComponent,
        entity_buffs::BuffsComponent,
        entity_info::EntityInfoComponent,
        entity_skills::{self, SkillsComponent},
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner,
        rally::{RallyPointComponent, RallyTarget},
        stance::StanceComponent,
        tags::TagsComponent,
        train::TrainQueueComponent,
    },
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    events::{SpawnCause, SpendCause},
    game_loop::{cast_cost, damage},
    input::InputFrames,
    map::Map,
    order::{AttackTarget, Order},
    player_buffs::PlayerBuffs,
    player_research::PlayerResearch,
    player_skills::{self, PlayerSkills},
    requirements,
    resources::{self, PlayerResources},
    selection::Selection,
    session::{GameSession, ai_vision::AiVision, player_id::PlayerId, player_slot::PlayerSlot},
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
    supply,
    visibility::VisibilityGrid,
};
use ferrets_content::{
    costs::Cost,
    entity_stats::EntityStatId,
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
            if interactable_entity(world, player, *id).is_some() {
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
                .filter(|&id| interactable_entity(world, player, id).is_some())
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
                && interactable_entity(world, player, id).is_none()
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
            if interactable_entity(world, player, *target).is_none() {
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
            if interactable_entity(world, player, *target).is_none() {
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
                && interactable_entity(world, player, *id).is_none()
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
        PlayerCommand::Repair { target, flush } => {
            if interactable_entity(world, player, *target).is_none() {
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
            if interactable_entity(world, player, *target).is_none() {
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
            if interactable_entity(world, player, *target).is_none() {
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
            if interactable_entity(world, player, *target).is_none() {
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

/// Validates and executes a train command: pays the cost up front and enqueues
/// the unit; production refunds on a force cancel.
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
    resources::charge(world, player, cost, SpendCause::Training { trainer });

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
    resources::charge(world, player, cost, SpendCause::Research { research });

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
            interactable_entity(world, player, id).is_some()
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

/// Resolves `id` to an entity `player` may name in a command: interactable —
/// alive and not hidden away inside something
/// ([`EntityIndex::interactable`]) — and, for a human player, standing in
/// its sight: the fog that hides a sprite must hide its stats and refuse
/// orders against it too. No ownership shortcut: own and allied entities
/// pass through the same grid (a unit's sight covers the cell it stands on,
/// and team vision is merged), so the grid stays the one truth.
///
/// A scripted player is gated by the vision its seat declares: a fog-limited
/// brain lives under the same rule as a human, an omniscient one legitimately
/// names what fog hides. The seat is session state, so every node (and a
/// replay) resolves its commands identically.
fn interactable_entity(world: &World, player: PlayerId, id: SimulationId) -> Option<Entity> {
    let entity = world.resource::<EntityIndex>().interactable(world, id)?;
    let session = world.resource::<GameSession>();
    let sight_gated = match session.slot(player).and_then(PlayerSlot::ai_vision) {
        None | Some(AiVision::Filtered) => true,
        Some(AiVision::Omniscient) => false,
    };
    if !sight_gated {
        return Some(entity);
    }
    let location = world.entity(entity).get::<LocationComponent>()?;
    world
        .resource::<VisibilityGrid>()
        .is_visible_to(
            session,
            player,
            location.position.x.to_num::<u32>(),
            location.position.y.to_num::<u32>(),
        )
        .then_some(entity)
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

/// Validates and executes a cast: the skill must exist, match its caster
/// kind, be off cooldown for that caster, be affordable (every cost payable),
/// and the target must be valid for the skill.
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
    // A named target must be in sight to be named at all — fog refuses the
    // cast the way it hides the sprite.
    if let Some(SkillTarget::Entity(target)) = target
        && interactable_entity(world, player, target).is_none()
    {
        return;
    }
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
                target: cast_target,
                effect,
            },
        ) => {
            use_skill_as_entity(
                world,
                player,
                skill,
                def.cooldown,
                &costs,
                cast_target,
                effect,
                caster_id,
                target,
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
    resources::charge(world, player, cost.clone(), SpendCause::Skill { skill });

    match effect {
        PlayerCastEffect::ApplyBuff(buff) => stats::apply_player_buff(world, player, buff),
        PlayerCastEffect::RemoveBuff(buff) => {
            world.resource_mut::<PlayerBuffs>().remove(player, buff);
        }
    }

    player_skills::cast(world, player, skill, cooldown);
}

/// Where a resolved entity cast lands.
#[derive(Clone, Copy)]
enum CastAim {
    /// On an entity.
    Entity(Entity),
    /// On a cell.
    Cell(CellPos),
}

/// The entity-cast path: the caster must be an owned entity whose type
/// declares the skill; cooldown per entity, pool costs draw from the caster,
/// the effect lands at the resolved aim.
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
    target: Option<SkillTarget>,
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

    // Only an operating caster casts.
    match entity_def::operation(world, caster) {
        Operation::Operating => {}
        Operation::UnderConstruction | Operation::Disabled => return,
    }

    // Resolve and validate the aim.
    let aim = match cast_target {
        EntityCastTarget::Caster => CastAim::Entity(caster),
        EntityCastTarget::Position => {
            let Some(SkillTarget::Position(position)) = target else {
                return;
            };
            let cell = CellPos::from(position);
            if !world.resource::<Map>().contains(cell) {
                return;
            }
            CastAim::Cell(cell)
        }
        EntityCastTarget::Ally | EntityCastTarget::Enemy => {
            let Some(SkillTarget::Entity(target_id)) = target else {
                return;
            };
            let Some(target) = world
                .resource::<EntityIndex>()
                .interactable(world, target_id)
            else {
                return;
            };
            let session = world.resource::<GameSession>();
            let caster_owner = entity_def::owner(world, caster);
            let target_owner = entity_def::owner(world, target);
            let valid = match cast_target {
                EntityCastTarget::Ally => matches!(
                    (caster_owner, target_owner),
                    (Some(caster), Some(target)) if session.are_allied(caster, target)
                ),
                EntityCastTarget::Enemy => owner::are_hostile(session, caster_owner, target_owner),
                EntityCastTarget::Caster | EntityCastTarget::Position => {
                    unreachable!("handled above")
                }
            };
            if !valid {
                return;
            }
            CastAim::Entity(target)
        }
    };

    if !cast_cost::can_pay(world, caster, player, costs) {
        return;
    }
    cast_cost::pay(world, caster, player, costs, SpendCause::Skill { skill });

    apply_skill_effect(world, player, caster, aim, effect);

    // A cast on a cell is announced against the caster, like a self-cast.
    let target_id = match aim {
        CastAim::Entity(target) => entity_def::simulation_id(world, target),
        CastAim::Cell(_) => caster_id,
    };
    entity_skills::cast(world, caster, target_id, skill, cooldown);
}

/// Applies a resolved skill effect at `aim`.
fn apply_skill_effect(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    aim: CastAim,
    effect: EntityCastEffect,
) {
    if let EntityCastEffect::Field {
        field,
        radius,
        action,
    } = effect
    {
        let center = match aim {
            CastAim::Cell(cell) => cell,
            CastAim::Entity(target) => CellPos::from(entity_def::position(world, target)),
        };
        super::fields::apply_action(world, player, field, center, radius, action);
        return;
    }
    let target = match aim {
        CastAim::Entity(target) => target,
        CastAim::Cell(_) => unreachable!("registration pairs a cell aim with a field effect only"),
    };
    match effect {
        EntityCastEffect::ApplyBuff(id) => super::stats::apply_entity_buff(world, target, id),
        EntityCastEffect::RemoveBuff(id) => {
            if let Some(mut buffs) = world.entity_mut(target).get_mut::<BuffsComponent>() {
                buffs.remove(id);
            }
        }
        EntityCastEffect::Damage(amount) => {
            // Skill damage bypasses armor, like an ability rather than a
            // weapon.
            let caster_id = entity_def::simulation_id(world, caster);
            damage::apply(world, caster_id, target, amount);
        }
        EntityCastEffect::Heal(amount) => {
            let max = entity_def::effective_stat(world, target, EntityStatId::MAX_HEALTH)
                .unwrap_or(FixedU64::ZERO);
            if let Some(mut health) = world.entity_mut(target).get_mut::<HealthComponent>() {
                health.heal(amount, max);
            }
        }
        EntityCastEffect::Field { .. } => unreachable!("handled above"),
    }
}
