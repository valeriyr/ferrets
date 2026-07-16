//! Simulation entity creation, destruction, and map presence.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};

use crate::{
    components::{
        dying::{CorpseComponent, DiedComponent, DyingComponent, DyingStaticData},
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::{LocationComponent, LocationStaticData},
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pending_reveal::PendingRevealComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        tags::TagsComponent,
        train::TrainQueueComponent,
    },
    content::registry::ContentRegistry,
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
        location_data,
        move_data,
        health_data,
        dying_data,
        attack_data,
        trainer,
        builder,
        resource_source,
        resource_carrier,
        resource_storage,
        tags,
    ) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name)?;
        (
            type_def.location?,
            type_def.movement,
            type_def.health,
            type_def.dying.clone(),
            type_def.attack,
            type_def.trainer.clone(),
            type_def.builder.clone(),
            type_def.resource_source.clone(),
            type_def.resource_carrier.clone(),
            type_def.resource_storage.clone(),
            type_def.tags.clone(),
        )
    };

    let location = LocationComponent::new(position, DEFAULT_FACING);

    {
        let map = world.resource::<Map>();
        if !map.can_place_entity(&location, &location_data) {
            return None;
        }
    }

    let id = world.resource_mut::<SimulationIdGenerator>().generate();

    let mut entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_name),
        location,
        location_data,
        OrderQueueComponent::default(),
    ));
    if let Some(player) = owner {
        entity_mut.insert(OwnerComponent::new(player));
    }
    if let Some(move_data) = move_data {
        entity_mut.insert(move_data);
    }
    if let Some(health_data) = health_data {
        entity_mut.insert((health_data, HealthComponent::full(&health_data)));
    }
    if let Some(dying_data) = dying_data {
        entity_mut.insert(dying_data);
    }
    if let Some(attack_data) = attack_data {
        entity_mut.insert(attack_data);
    }
    if let Some(train_data) = trainer {
        entity_mut.insert((train_data, TrainQueueComponent::default()));
    }
    if let Some(builder_data) = builder {
        entity_mut.insert(builder_data);
    }
    if let Some(source_data) = resource_source {
        entity_mut.insert((source_data, ResourceSourceComponent::default()));
    }
    if let Some(carrier_data) = resource_carrier {
        entity_mut.insert((carrier_data, ResourceCarrierComponent::default()));
    }
    if let Some(storage_data) = resource_storage {
        entity_mut.insert(storage_data);
    }
    if !tags.is_empty() {
        entity_mut.insert(TagsComponent::new(tags));
    }
    let entity = entity_mut.id();

    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_data);
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
    let (location_data, dying_data) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name)?;
        (type_def.location?, type_def.dying.clone())
    };

    let location = LocationComponent::new(position, DEFAULT_FACING);
    if !world
        .resource::<Map>()
        .can_place_entity(&location, &location_data)
    {
        return None;
    }
    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_data);

    let id = world.resource_mut::<SimulationIdGenerator>().generate();
    let dying_time = dying_data.as_ref().map(|d| d.dying_time()).unwrap_or(0);

    let mut queue = OrderQueueComponent::default();
    queue.push(Order::Die, None);

    let mut entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_name),
        location,
        location_data,
        queue,
        CorpseComponent,
        DyingComponent {
            ticks_remaining: dying_time,
        },
    ));
    if let Some(dying_data) = dying_data {
        entity_mut.insert(dying_data);
    }
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
    let location_data = *world
        .entity(entity)
        .get::<LocationStaticData>()
        .expect("only entities with LocationStaticData can be hidden");
    world
        .resource_mut::<Map>()
        .displace_entity(&location, &location_data);
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

    let location_data = *world
        .entity(entity)
        .get::<LocationStaticData>()
        .expect("only entities with LocationStaticData can be revealed");
    let Some(cell) =
        world
            .resource::<Map>()
            .find_placement_near(around, around_size, &location_data)
    else {
        return false;
    };

    place_hidden_at(world, entity, cell, &location_data);
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
fn place_hidden_at(
    world: &mut World,
    entity: Entity,
    cell: NavPos,
    location_data: &LocationStaticData,
) {
    let mut entity_mut = world.entity_mut(entity);

    entity_mut.remove::<HiddenComponent>();

    let mut location = entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can be revealed");
    location.position = FixedUVec2::from(cell);

    let location = *location;
    world
        .resource_mut::<Map>()
        .place_entity(&location, location_data);
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

    let dying_time = world
        .entity(entity)
        .get::<DyingStaticData>()
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
