//! Simulation entity creation, destruction, and map presence.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};

use crate::{
    components::{
        dying::{CorpseComponent, DiedComponent, DyingComponent},
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pending_reveal::PendingRevealComponent,
        rally::RallyPointComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        skills::SkillsComponent,
        stance::{Stance, StanceComponent},
        stats::StatsComponent,
        tags::TagsComponent,
        train::TrainQueueComponent,
    },
    content::{
        stats::StatId,
        {location::LocationDef, registry::ContentRegistry},
    },
    control_groups::ControlGroups,
    entity_def,
    entity_index::EntityIndex,
    game_loop::movement::is_mid_crossing,
    map::Map,
    order::Order,
    selection::Selection,
    session::player_slot::PlayerId,
    simulation_id::{SimulationId, SimulationIdGenerator},
};

/// Look direction a freshly spawned entity starts with: south, `+y` (sim `y`
/// points down), the conventional resting facing toward the viewer.
const DEFAULT_FACING: FixedVec2 = FixedVec2::new(FixedI64::ZERO, FixedI64::ONE);

/// Spawns an entity of the given type at `position`, owned by `owner`
/// (`None` spawns a neutral entity).
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not registered
/// or the position is blocked on the nav grid.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
) -> Option<(Entity, SimulationId)> {
    let (
        type_id,
        location_def,
        base_stats,
        is_attacker,
        is_mover,
        is_damageable,
        has_trainer,
        has_resource_source,
        has_resource_carrier,
        tags,
        skills,
    ) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        (
            type_id,
            type_def.location?,
            type_def.base_stats.clone(),
            type_def.can_attack(),
            type_def.can_move(),
            type_def.is_damageable(),
            type_def.trainer.is_some(),
            type_def.resource_source.is_some(),
            type_def.resource_carrier.is_some(),
            type_def.tags.clone(),
            type_def.skills.clone(),
        )
    };

    let location = LocationComponent::new(position, DEFAULT_FACING);

    {
        let map = world.resource::<Map>();
        if !map.can_place_entity(&location, &location_def) {
            return None;
        }
    }

    let id = world.resource_mut::<SimulationIdGenerator>().generate();

    let mut entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_id, type_name),
        location,
        OrderQueueComponent::default(),
    ));
    if let Some(player) = owner {
        entity_mut.insert(OwnerComponent::new(player));
    }
    // Seed the per-entity stat store from the type's base stats — built-in and
    // custom alike. Buffs later fold these into `effective` (see
    // game_loop::stats::recompute_stats).
    let mut stats = StatsComponent::default();
    for (&stat, &value) in &base_stats {
        stats.set_base(stat, value);
    }
    entity_mut.insert(stats);

    // Current-value pools, seeded to full from their max stats.
    if let Some(&max_health) = base_stats.get(&StatId::MAX_HEALTH) {
        entity_mut.insert(HealthComponent::full(max_health));
    }
    if let Some(&max_energy) = base_stats.get(&StatId::MAX_ENERGY) {
        entity_mut.insert(EnergyComponent::full(max_energy));
    }

    // Stance: armed entities default to defending themselves; unarmed but movable,
    // damageable ones to fleeing; the rest have no initiative to configure.
    if is_attacker {
        entity_mut.insert(StanceComponent(Stance::Defend));
    } else if is_mover && is_damageable {
        entity_mut.insert(StanceComponent(Stance::Flee));
    }

    // Production and resource roles get their runtime-state components; the
    // type-constant config stays on the definition, read via its handle.
    if has_trainer {
        entity_mut.insert((
            TrainQueueComponent::default(),
            RallyPointComponent::default(),
        ));
    }
    if has_resource_source {
        entity_mut.insert(ResourceSourceComponent::default());
    }
    if has_resource_carrier {
        entity_mut.insert(ResourceCarrierComponent::default());
    }
    if !tags.is_empty() {
        entity_mut.insert(TagsComponent::new(tags));
    }
    if !skills.is_empty() {
        entity_mut.insert(SkillsComponent::new(skills));
    }

    let entity = entity_mut.id();

    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_def);
    world.resource_mut::<EntityIndex>().insert_alive(id, entity);

    Some((entity, id))
}

/// Spawns an entity of the given type as remains, directly in the dying state,
/// at `position`.
///
/// The entity never joins the alive world: it is not selectable or targetable,
/// and is removed when its dying phase completes. Remains always occupy the
/// navigation grid per their occupation mask — blocking rubble claims movement
/// layers, walkable corpses use a layer moving entities do not collide with —
/// so when the footprint cells are not free, no remains are left at all.
///
/// Only the components meaningful in the dying state are added: identity,
/// location, the [`CorpseComponent`] marker, the order queue with its `Die`
/// order, and the dying properties. Live-gameplay components from the type
/// definition (health, movement, combat, …) are skipped — remains can never
/// use them, and a freshly initialized value (e.g. full health on a corpse)
/// would be false.
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not
/// registered or the footprint is blocked.
pub fn spawn_corpse_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
) -> Option<(Entity, SimulationId)> {
    let (type_id, location_def, dying_def) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        (type_id, type_def.location?, type_def.dying.clone())
    };

    let location = LocationComponent::new(position, DEFAULT_FACING);
    if !world
        .resource::<Map>()
        .can_place_entity(&location, &location_def)
    {
        return None;
    }
    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_def);

    let id = world.resource_mut::<SimulationIdGenerator>().generate();
    let dying_time = dying_def.as_ref().map(|d| d.dying_time()).unwrap_or(0);

    let mut queue = OrderQueueComponent::default();
    queue.push(Order::Die, None);

    let entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_id, type_name),
        location,
        queue,
        CorpseComponent,
        DyingComponent {
            ticks_remaining: dying_time,
        },
    ));
    let entity = entity_mut.id();

    world.resource_mut::<EntityIndex>().insert_dying(id, entity);

    Some((entity, id))
}

/// Takes an entity off the map: frees its footprint and marks it hidden.
///
/// A hidden entity cannot be selected or targeted and holds no cells. Bring it
/// back with [`reveal_entity_near`] or [`reveal_entity_near_or_retry`].
pub(crate) fn hide_entity(world: &mut World, entity: Entity) {
    let location = *world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("only entities with LocationComponent can be hidden");
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can be hidden");
    world
        .resource_mut::<Map>()
        .displace_entity(&location, &location_def);
    // Hiding is the inverse of a pending reveal: drop any stale retry so a new
    // hide is not undone by `process_pending_reveals` on a later tick.
    world
        .entity_mut(entity)
        .remove::<PendingRevealComponent>()
        .insert(HiddenComponent);
}

/// Puts a hidden entity back on the map on the nearest free cell around the
/// footprint at `around`/`around_size`.
///
/// Returns `false` when no free cell exists within the search radius — the
/// entity stays hidden, and the caller is expected to retry. Returns `true`
/// for entities that are not hidden.
pub(crate) fn reveal_entity_near(
    world: &mut World,
    entity: Entity,
    around: NavPos,
    around_size: NavSize,
) -> bool {
    if !world.entity(entity).contains::<HiddenComponent>() {
        return true;
    }

    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can be revealed");
    let Some(cell) =
        world
            .resource::<Map>()
            .find_placement_near(around, around_size, &location_def)
    else {
        return false;
    };

    place_hidden_at(world, entity, cell, &location_def);
    true
}

/// Like [`reveal_entity_near`], but for callers that cannot retry themselves
/// (order cancellation and other paths that finish in the same tick).
///
/// Reveals immediately when a cell is free. Otherwise the entity stays hidden
/// and is tagged with [`PendingRevealComponent`], so
/// [`game_loop::pending_reveal::process_pending_reveals`] keeps retrying every
/// tick until a cell opens — rather than forcing it onto an occupied cell and
/// corrupting the nav grid.
pub(crate) fn reveal_entity_near_or_retry(
    world: &mut World,
    entity: Entity,
    around: NavPos,
    around_size: NavSize,
) {
    if reveal_entity_near(world, entity, around, around_size) {
        return;
    }

    world.entity_mut(entity).insert(PendingRevealComponent {
        around,
        around_size,
    });
}

/// Puts a hidden entity back on the map at `cell`.
fn place_hidden_at(world: &mut World, entity: Entity, cell: NavPos, location_def: &LocationDef) {
    let mut entity_mut = world.entity_mut(entity);

    entity_mut.remove::<HiddenComponent>();

    let mut location = entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can be revealed");
    location.position = FixedUVec2::from(cell);

    let location = *location;
    world
        .resource_mut::<Map>()
        .place_entity(&location, location_def);
}

/// Starts the dying phase for an alive entity.
///
/// The entity immediately leaves the alive set: it is removed from every
/// player's selection, all queued orders are force-cancelled, and a `Die` order
/// is queued. The entity stays in the world as dying — still holding its
/// footprint on the nav grid — until the `Die` order completes and frees it.
///
/// No-op if the entity is already dying or has died.
pub fn destroy_entity(world: &mut World, entity: Entity) {
    {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<DyingComponent>() || entity_ref.contains::<DiedComponent>() {
            return;
        }
    }

    let id = world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .id();
    let location = *world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("simulation entity must have LocationComponent");

    // The entity keeps its footprint through the dying phase, but the movement
    // state that knows which cell a crossing claimed is about to be cancelled —
    // so a mid-crossing entity snaps onto its claimed cell.
    if is_mid_crossing(location.position)
        && let Some(claimed) = world
            .entity(entity)
            .get::<MoveComponent>()
            .and_then(|mc| mc.path.last().copied())
    {
        world
            .entity_mut(entity)
            .get_mut::<LocationComponent>()
            .expect("only entities with LocationComponent can be moving")
            .position = FixedUVec2::from(claimed);
    }

    world.resource_mut::<Selection>().remove(id);
    world.resource_mut::<ControlGroups>().remove(id);

    let dying_time = entity_def::of(world, entity)
        .dying
        .as_ref()
        .map(|d| d.dying_time())
        .unwrap_or(0);

    let mut entity_mut = world.entity_mut(entity);
    entity_mut.insert(DyingComponent {
        ticks_remaining: dying_time,
    });
    if let Some(mut queue) = entity_mut.get_mut::<OrderQueueComponent>() {
        queue.push(Order::Die, Some(CancelPolicy::Force));
    }

    world.resource_mut::<EntityIndex>().mark_dying(id);
}

/// Removes a dead entity from the world after its dying phase has completed.
///
/// Panics if the entity has not finished dying.
pub fn remove_dead_entity(world: &mut World, entity: Entity) {
    debug_assert!(
        world.entity(entity).contains::<DiedComponent>(),
        "remove_dead_entity requires a finished dying phase"
    );

    let id = world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .id();

    world.resource_mut::<EntityIndex>().remove_dying(id);
    world.despawn(entity);
}
