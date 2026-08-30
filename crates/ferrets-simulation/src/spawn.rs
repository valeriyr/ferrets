//! Simulation entity creation, destruction, and map presence.

use std::collections::BTreeMap;

use bevy_ecs::{component::Component, entity::Entity, world::EntityWorldMut, world::World};
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeId, location::LocationDef,
    registry::ContentRegistry, transport::PassengerFate,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};

use crate::{
    components::{
        dying::{CorpseComponent, DiedComponent, DyingComponent},
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pending_reveal::PendingRevealComponent,
        rally::RallyPointComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::{Stance, StanceComponent},
        tags::TagsComponent,
        train::TrainQueueComponent,
        transport::{BoardedComponent, GarrisonFireComponent, TransporterComponent},
        turret::{TurretState, TurretsComponent},
    },
    control_groups::ControlGroups,
    entity_def,
    entity_index::EntityIndex,
    game_loop::movement::is_mid_crossing,
    map::{Map, OccupancyClass},
    movement_model::MovementModel,
    order::Order,
    selection::Selection,
    session::player_slot::PlayerId,
    simulation_id::{SimulationId, SimulationIdGenerator},
};
/// Look direction a freshly spawned entity starts with: south, the conventional
/// resting facing toward the viewer.
pub(crate) const DEFAULT_FACING: Facing = Facing::SOUTH;

/// Spawns an entity of the given type at `position`, owned by `owner`
/// (`None` spawns a neutral entity).
///
/// `position` must lie exactly on a cell's origin corner — a fresh entity is
/// at rest, and rest positions are lattice points.
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not registered
/// or the position is blocked on the nav grid.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
) -> Option<(Entity, SimulationId)> {
    debug_assert!(
        !is_mid_crossing(position),
        "entities spawn at rest: position must lie exactly on a cell origin"
    );
    // Only what placing the entity needs; every capability component is fitted
    // from the type by `fit_components` below.
    let (type_id, location_def, base_stats) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        (type_id, type_def.location?, type_def.base_stats.clone())
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
    let entity = entity_mut.id();

    seed_stats(world, entity, &base_stats);
    // Current-value pools, seeded to full from their max stats. A morph rescales
    // them instead, which is why filling them is the spawn's own business.
    if let Some(&max_health) = base_stats.get(&EntityStatId::MAX_HEALTH) {
        world
            .entity_mut(entity)
            .insert(HealthComponent::full(max_health));
    }
    if let Some(&max_energy) = base_stats.get(&EntityStatId::MAX_ENERGY) {
        world
            .entity_mut(entity)
            .insert(EnergyComponent::full(max_energy));
    }
    fit_components(world, entity, type_id);

    let class = OccupancyClass::of(world.resource::<ContentRegistry>().def(type_id));
    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_def, class);
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
///
/// `position` must lie exactly on a cell's origin corner, like every rest
/// position.
pub fn spawn_corpse_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
) -> Option<(Entity, SimulationId)> {
    debug_assert!(
        !is_mid_crossing(position),
        "remains spawn at rest: position must lie exactly on a cell origin"
    );
    let (type_id, location_def, dying_def, class) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        (
            type_id,
            type_def.location?,
            type_def.dying.clone(),
            OccupancyClass::of(type_def),
        )
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
        .place_entity(&location, &location_def, class);

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
///
/// Safe to call on an entity that is already hidden — one stuck waiting on a free
/// cell can be hidden again by whatever it does next.
pub(crate) fn hide_entity(world: &mut World, entity: Entity) {
    // An entity that is already off the map holds no cells to free. Freeing them
    // again would clear whatever moved onto them while it was away — and the marker
    // below carries nothing, so setting it twice costs nothing either.
    if !world.entity(entity).contains::<HiddenComponent>() {
        let location = *world
            .entity(entity)
            .get::<LocationComponent>()
            .expect("only entities with LocationComponent can be hidden");
        let def = entity_def::of(world, entity);
        let location_def = def
            .location
            .expect("only entities with LocationDef can be hidden");
        let class = OccupancyClass::of(def);
        world
            .resource_mut::<Map>()
            .displace_entity(&location, &location_def, class);
    }
    // Hiding is the inverse of a pending reveal: drop any stale retry so a new hide
    // is not undone by `process_pending_reveals` on a later tick. This is the part
    // that still matters for an entity that was already hidden — it is off the map
    // for a new reason now, and comes back where that reason says rather than where
    // the abandoned one would have put it.
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
    around: CellPos,
    around_size: CellSize,
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
    around: CellPos,
    around_size: CellSize,
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
fn place_hidden_at(world: &mut World, entity: Entity, cell: CellPos, location_def: &LocationDef) {
    let class = OccupancyClass::of(entity_def::of(world, entity));
    let mut entity_mut = world.entity_mut(entity);

    entity_mut.remove::<HiddenComponent>();

    let mut location = entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can be revealed");
    location.position = FixedUVec2::from(cell);

    let location = *location;
    world
        .resource_mut::<Map>()
        .place_entity(&location, location_def, class);
}

/// Starts the dying phase for an alive entity.
///
/// The entity immediately leaves the alive set: it is removed from every
/// player's selection, all queued orders are force-cancelled, and a `Die` order
/// is queued. The entity stays in the world as dying — still holding its
/// footprint on the nav grid under the cell model, while the continuous
/// rebuild stops counting its body — until the `Die` order completes and
/// frees it.
///
/// No-op if the entity is already dying or has died.
pub fn destroy_entity(world: &mut World, entity: Entity) {
    {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<DyingComponent>() || entity_ref.contains::<DiedComponent>() {
            return;
        }
    }

    let id = entity_def::simulation_id(world, entity);
    let location = *world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("simulation entity must have LocationComponent");

    // The entity keeps its footprint through the dying phase, but the movement
    // state that knows which cell a crossing claimed is about to be cancelled —
    // so a mid-crossing entity snaps onto its claimed cell. Continuous
    // movers die where they stand: their positions are free points and their
    // claim already tracks the floored cell.
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => {
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
        }
        MovementModel::Continuous => {}
    }

    world.resource_mut::<Selection>().remove(id);
    world.resource_mut::<ControlGroups>().remove(id);

    settle_passengers(world, entity);
    leave_holder(world, entity, id);

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

/// Applies a dying transporter's declared passenger fate to everyone aboard.
///
/// Ejection is one placement attempt per passenger, in id order so earlier ids
/// take the closer cells on every peer; a passenger the ring scan cannot place
/// dies with its holder rather than lingering hidden with nothing holding it.
fn settle_passengers(world: &mut World, entity: Entity) {
    let Some(passengers) = world
        .entity_mut(entity)
        .get_mut::<TransporterComponent>()
        .map(|mut transporter| std::mem::take(&mut transporter.passengers))
    else {
        return;
    };
    if passengers.is_empty() {
        return;
    }
    let fate = entity_def::of(world, entity)
        .transporter
        .as_ref()
        .expect("a passenger list belongs to a transporter")
        .passenger_fate();
    let (around, around_size) = entity_def::footprint(world, entity);
    let around = CellPos::from(around);

    for id in passengers {
        let Some(passenger) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        world
            .entity_mut(passenger)
            .remove::<(BoardedComponent, GarrisonFireComponent)>();
        match fate {
            PassengerFate::Destroy => destroy_entity(world, passenger),
            PassengerFate::Eject => {
                if !reveal_entity_near(world, passenger, around, around_size) {
                    destroy_entity(world, passenger);
                }
            }
        }
    }
}

/// Takes a dying passenger off its holder's list, freeing the slots it held.
fn leave_holder(world: &mut World, entity: Entity, id: SimulationId) {
    let Some(boarded) = world.entity_mut(entity).take::<BoardedComponent>() else {
        return;
    };
    if let Some(holder) = world.resource::<EntityIndex>().alive(boarded.holder)
        && let Some(mut transporter) = world.entity_mut(holder).get_mut::<TransporterComponent>()
    {
        transporter.passengers.remove(&id);
    }
}

/// Removes a dead entity from the world after its dying phase has completed.
///
/// Panics if the entity has not finished dying.
pub fn remove_dead_entity(world: &mut World, entity: Entity) {
    debug_assert!(
        world.entity(entity).contains::<DiedComponent>(),
        "remove_dead_entity requires a finished dying phase"
    );

    let id = entity_def::simulation_id(world, entity);

    world.resource_mut::<EntityIndex>().remove_dying(id);
    world.despawn(entity);
}

/// Seeds `entity`'s stat store from a type's base stats — built-in and custom
/// alike. Buffs later fold these into `effective` (see
/// [`game_loop::stats::recompute_entity_stats`](crate::game_loop::stats::recompute_entity_stats)).
///
/// Replaces the whole store, because bases belong to the type: a type change
/// must not leave a stat the old type carried and the new one does not.
pub(crate) fn seed_stats(
    world: &mut World,
    entity: Entity,
    base_stats: &BTreeMap<EntityStatId, FixedU64>,
) {
    let mut stats = StatsComponent::default();
    for (&stat, &value) in base_stats {
        stats.set_base(stat, value);
    }
    world.entity_mut(entity).insert(stats);
}

/// Fits `entity`'s components to what `type_id` requires: inserts the ones the
/// type needs and removes the ones it no longer does.
///
/// **Live state on a component both types need is left alone.** A holder keeps
/// its passengers through a type change, a trainer its queue, a carrier its
/// load — inserting a fresh default would silently empty them, which is the
/// quietest way this could go wrong.
///
/// Stance is preserved when present, because a player sets it deliberately;
/// only an entity that has none is given its type's default.
pub(crate) fn fit_components(world: &mut World, entity: Entity, type_id: EntityTypeId) {
    let (
        can_attack,
        mounted_turrets,
        can_move,
        has_health,
        trainer,
        transporter,
        source,
        carrier,
        tags,
        skills,
    ) = {
        let def = world.resource::<ContentRegistry>().def(type_id);
        (
            def.can_attack(),
            def.turrets.len(),
            def.can_move(),
            def.has_health(),
            def.trainer.is_some(),
            def.can_transport(),
            def.resource_source.is_some(),
            def.resource_carrier.is_some(),
            def.tags.clone(),
            def.skills.clone(),
        )
    };
    // A rally point serves the trainer and the holder alike, so it stays while
    // either role does.
    let wants_rally = trainer || transporter;
    let mut entity_mut = world.entity_mut(entity);

    // Armed entities default to defending themselves; unarmed but movable,
    // damageable ones to fleeing; the rest have no initiative to configure.
    if !entity_mut.contains::<StanceComponent>() {
        if can_attack {
            entity_mut.insert(StanceComponent(Stance::Defend));
        } else if can_move && has_health {
            entity_mut.insert(StanceComponent(Stance::Flee));
        }
    }

    // Runtime-state components for the roles the type carries; the
    // type-constant config stays on the definition, read via its handle.
    //
    // Losing a role drops its state where it stands, which is right for
    // everything except a queue whose entries were paid for up front: a
    // trainer that becomes something else would take its unbuilt units with
    // it, unrefunded. The order lifecycle owns that refund — a flushed Train
    // order gives every entry back — so whatever arrives here has already
    // been emptied, and debug builds hold the lifecycle to it.
    debug_assert!(
        trainer
            || entity_mut
                .get::<TrainQueueComponent>()
                .is_none_or(|queue| queue.0.is_empty()),
        "a type change must not drop a paid production queue"
    );
    fit_default::<TrainQueueComponent>(&mut entity_mut, trainer);
    fit_default::<TransporterComponent>(&mut entity_mut, transporter);
    fit_default::<RallyPointComponent>(&mut entity_mut, wants_rally);
    fit_default::<ResourceSourceComponent>(&mut entity_mut, source);
    fit_default::<ResourceCarrierComponent>(&mut entity_mut, carrier);

    // A turret remembers where it is trained, which no other component can hold
    // for it: a fight's state is gone the moment the fight ends, and the body of a
    // keep never turns. Mounted looking the way its body does, and left where it
    // was last trained through any change that keeps the mount — a form with more
    // guns than the last one trains the new ones forward, and one with fewer drops
    // the guns it no longer has.
    if mounted_turrets == 0 {
        entity_mut.remove::<TurretsComponent>();
    } else {
        let facing = entity_mut
            .get::<LocationComponent>()
            .expect("a placed entity has a location")
            .facing;
        let mut turrets = entity_mut.take::<TurretsComponent>().unwrap_or_default();
        turrets
            .0
            .resize(mounted_turrets, TurretState::mounted(facing));
        entity_mut.insert(turrets);
    }

    // Tags and skills are the type's own vocabulary rather than live state, so
    // they are replaced outright.
    if tags.is_empty() {
        entity_mut.remove::<TagsComponent>();
    } else {
        entity_mut.insert(TagsComponent::new(tags));
    }
    if skills.is_empty() {
        entity_mut.remove::<SkillsComponent>();
    } else {
        entity_mut.insert(SkillsComponent::new(skills));
    }
}

/// Inserts a default `C` when the type wants one and it is absent, removes it
/// when the type does not, and leaves an existing one untouched — so whatever
/// live state it holds survives.
fn fit_default<C: Component + Default>(entity: &mut EntityWorldMut, wanted: bool) {
    match (wanted, entity.contains::<C>()) {
        (true, false) => {
            entity.insert(C::default());
        }
        (false, true) => {
            entity.remove::<C>();
        }
        (true, true) | (false, false) => {}
    }
}
