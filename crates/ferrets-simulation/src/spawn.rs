//! Simulation entity creation, destruction, and map presence.

use std::collections::BTreeMap;

use bevy_ecs::{component::Component, entity::Entity, world::EntityWorldMut, world::World};
use ferrets_content::{
    brood::{BreederDef, OrphanFate},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeId,
    location::LocationDef,
    morph::MorphReason,
    registry::ContentRegistry,
    resource::DepletionPolicy,
    transport::PassengerFate,
    work::Attachment,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use crate::{
    berths, brood,
    components::{
        annex::{AnnexComponent, DocksComponent},
        attached::AttachedComponent,
        brood::{BredComponent, BroodComponent},
        build::OverbuiltComponent,
        dying::{CorpseComponent, DiedComponent, DyingComponent},
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        field_source::FieldSourcesComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        morph::MorphComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pending_reveal::PendingRevealComponent,
        rally::RallyPointComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::{Stance, StanceComponent},
        stand::{StandComponent, Standing},
        tags::TagsComponent,
        train::TrainQueueComponent,
        transport::{BoardedComponent, GarrisonFireComponent, TransporterComponent},
        turret::{TurretState, TurretsComponent},
    },
    control_groups::ControlGroups,
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause},
    map::{Map, OccupancyClass},
    movement_model::{self, MovementModel},
    order::Order,
    selection::Selection,
    session::player_id::PlayerId,
    simulation_id::{SimulationId, SimulationIdGenerator},
};
/// Look direction a freshly spawned entity starts with: south, the conventional
/// resting facing toward the viewer.
pub(crate) const DEFAULT_FACING: Facing = Facing::SOUTH;

/// Which form a refit puts on an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wearing {
    /// A form of its own: a fresh entity's, a landed change's, or the origin
    /// a change returns to.
    Own,
    /// An interim form, worn while a change of form runs.
    Interim,
}

/// What fitting a type does about the acts it performs on standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingActs {
    /// The acts lie ahead again: the entity is coming to stand as something
    /// new.
    Rearm,
    /// Acts already performed stay performed; only a form that had none and
    /// gains some has them ahead.
    Keep,
}

/// How far an entity's field sources reach the moment it enters the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldReach {
    /// Each source starts at its declared initial reach and grows from there.
    Initial,
    /// Each source already spans its whole radius.
    Full,
}

/// Creates an entity of the given type at `position`, owned by `owner`
/// (`None` creates a neutral entity), its field sources reaching as `reach`
/// says, announcing nothing.
///
/// `position` must lie exactly on a cell's origin corner — a fresh entity is
/// at rest, and rest positions are lattice points.
///
/// [`spawn_entity`] is the announcing counterpart.
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not registered
/// or the position is blocked on the nav grid.
pub fn create_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
) -> Option<(Entity, SimulationId)> {
    debug_assert!(
        !movement_model::is_mid_crossing(position),
        "entities spawn at rest: position must lie exactly on a cell origin"
    );
    let location_def = world
        .resource::<ContentRegistry>()
        .entity(type_name)?
        .location?;
    let location = LocationComponent::new(position, DEFAULT_FACING);
    if !world
        .resource::<Map>()
        .can_place_entity(&location, &location_def)
    {
        return None;
    }

    let (entity, id) = conjure(world, type_name, location, owner, reach)?;
    // Stamped from the location the entity now carries, not the one tested
    // above.
    restore_footprint(world, entity);
    Some((entity, id))
}

/// Brings an entity of the given type into the world at `location`, owned by
/// `owner`, with everything its type gives an instance — and no footprint on
/// the grid: the caller stands it on the grid or seats it. Returns `None` if
/// `type_name` is not registered or has no location.
fn conjure(
    world: &mut World,
    type_name: &str,
    location: LocationComponent,
    owner: Option<PlayerId>,
    reach: FieldReach,
) -> Option<(Entity, SimulationId)> {
    // Only what standing the entity needs; every capability component is fitted
    // from the type by `fit_components` below.
    let (type_id, base_stats) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        type_def.location?;
        (type_id, type_def.base_stats.clone())
    };

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
    fit_components(
        world,
        entity,
        type_id,
        reach,
        StandingActs::Rearm,
        Wearing::Own,
    );
    brood::open(world, entity);
    world.resource_mut::<EntityIndex>().insert_alive(id, entity);

    Some((entity, id))
}

/// Creates an entity of the given type seated in a berth of `job`'s group
/// named by `attachment`, owned by `owner`, and announces the spawn. The
/// entity never stands on the grid on its way to the seat.
///
/// Returns `None` if `type_name` is not registered or has no location, or the
/// group has no free berth.
pub(crate) fn spawn_seated(
    world: &mut World,
    type_name: &str,
    job: Entity,
    attachment: &Attachment,
    owner: Option<PlayerId>,
    cause: SpawnCause,
) -> Option<(Entity, SimulationId)> {
    let from = entity_def::footprint_rect(world, job);
    let (seat, point) = berths::nearest_free_seat(world, job, attachment.berths(), from)?;
    // Facing the way every fresh entity does; the seat sets the position.
    let location = LocationComponent::new(point, DEFAULT_FACING);
    let (entity, id) = conjure(world, type_name, location, owner, FieldReach::Initial)
        .expect("validated content breeds a registered type with a location");
    seat_at(world, entity, job, attachment, seat, point);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::EntitySpawned { entity: id, cause });
    Some((entity, id))
}

/// Creates an entity of the given type and announces the spawn.
///
/// The announcing counterpart to [`create_entity`]. `cause` travels on the
/// announcement, where a tally can tell a trained unit from a placed one.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    cause: SpawnCause,
    reach: FieldReach,
) -> Option<(Entity, SimulationId)> {
    let (entity, id) = create_entity(world, type_name, position, owner, reach)?;
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::EntitySpawned { entity: id, cause });
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
pub(crate) fn spawn_corpse_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    of: SimulationId,
) -> Option<(Entity, SimulationId)> {
    debug_assert!(
        !movement_model::is_mid_crossing(position),
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
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::EntitySpawned {
            entity: id,
            cause: SpawnCause::Remains { of },
        });

    Some((entity, id))
}

/// Takes an entity off the map: frees its footprint and marks it hidden.
///
/// A hidden entity cannot be selected or targeted and holds no cells. Bring it
/// back with [`place_back_near`] or [`place_back_near_or_retry`].
///
/// Safe to call on an entity that is already hidden — one stuck waiting on a free
/// cell can be hidden again by whatever it does next — and on one attached to
/// a job, which holds no cells to free.
pub(crate) fn hide_entity(world: &mut World, entity: Entity) {
    if !world.entity(entity).contains::<HiddenComponent>() {
        let announced = SimulationEvent::EntityHidden {
            entity: entity_def::simulation_id(world, entity),
        };
        world.resource_mut::<EventRecord>().emit(announced);
        lift_footprint(world, entity);
    }
    unseat(world, entity);
    // Hiding is the inverse of a pending return: drop any stale retry so a new hide
    // is not undone by `process_pending_reveals` on a later tick. This is the part
    // that still matters for an entity that was already hidden — it is off the map
    // for a new reason now, and comes back where that reason says rather than where
    // the abandoned one would have put it.
    world
        .entity_mut(entity)
        .remove::<PendingRevealComponent>()
        .insert(HiddenComponent);
}

/// Seats an entity in a berth of `job`'s group named by `attachment`: it frees
/// the cells it held, puts its middle on the free berth nearest to where it
/// stood, and holds no cells from there on while staying on the map. Bring it back onto
/// the grid with [`place_back_near`] or [`place_back_near_or_retry`].
///
/// Panics for a hidden entity, which has nothing to stand on, and when the
/// group has no free berth: a caller attaches only where [`berths::shut`]
/// found room.
pub(crate) fn attach_entity(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
) {
    assert!(
        !world.entity(entity).contains::<HiddenComponent>(),
        "only an entity on the map attaches to a job"
    );
    lift_footprint(world, entity);
    // A worker already seated elsewhere gives that berth up first.
    unseat(world, entity);
    seat_unseated(world, entity, job, attachment);
}

/// Seats an entity that holds no cells and no berth — one that has just given
/// its seat up through [`unseat`] — in the free berth of `job`'s group named
/// by `attachment` nearest to where it is.
///
/// Panics when the group has no free berth: a caller seats only where it
/// found room.
pub(crate) fn seat_unseated(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
) {
    let standing = entity_def::standing_rect(world, entity);
    let (seat, point) = berths::nearest_free_seat(world, job, attachment.berths(), standing)
        .expect("a worker attaches only to a job with a free berth in its group");
    seat_at(world, entity, job, attachment, seat, point);
}

/// Sits an entity off the grid down in berth `seat` of `job`'s group named by
/// `attachment`, at `point`: the berth is taken and the entity's middle put on
/// the point.
fn seat_at(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
    seat: usize,
    point: FixedUVec2,
) {
    let id = entity_def::simulation_id(world, entity);
    let job_id = entity_def::simulation_id(world, job);
    berths::occupy(world, job, attachment.berths(), seat, id);

    let position = berths::position_centred_on(world, entity, point);
    let mut entity_mut = world.entity_mut(entity);
    entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can attach")
        .position = position;
    entity_mut
        .remove::<PendingRevealComponent>()
        .insert(AttachedComponent {
            job: job_id,
            berths: attachment.berths().to_string(),
            seat,
            stance: attachment.stance(),
            sitting: berths::sit_down(attachment.stance(), id),
            hops: 0,
        });
}

/// Takes an entity out of the berth it sits in, if any, giving the berth back
/// to the job. The entity is otherwise untouched, standing where its berth
/// was with nothing under it.
pub(crate) fn unseat(world: &mut World, entity: Entity) -> Option<AttachedComponent> {
    let attached = world.entity_mut(entity).take::<AttachedComponent>()?;
    berths::vacate(world, &attached);
    Some(attached)
}

/// Frees the footprint an entity holds on the grid. An entity already off the
/// grid — hidden, or attached to a job — holds nothing to free.
///
/// The entity is otherwise untouched, so a caller that lifts a footprint to
/// test the ground under it puts it back with [`restore_footprint`] or takes
/// the entity off the map itself.
pub(crate) fn lift_footprint(world: &mut World, entity: Entity) {
    if !entity_def::stands_on_grid(world, entity) {
        return;
    }
    let location = *world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("only entities with LocationComponent hold a footprint");
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef hold a footprint");
    let class = OccupancyClass::of(def);
    world
        .resource_mut::<Map>()
        .displace_entity(&location, &location_def, class);
}

/// Stamps back the footprint [`lift_footprint`] took off an entity that
/// otherwise stands on the grid.
pub(crate) fn restore_footprint(world: &mut World, entity: Entity) {
    if !entity_def::stands_on_grid(world, entity) {
        return;
    }
    let location = *world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("only entities with LocationComponent hold a footprint");
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef hold a footprint");
    let class = OccupancyClass::of(def);
    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_def, class);
}

/// Puts an entity that is off the grid back on it, on the nearest free cell:
/// a hidden one around the footprint at `around`/`around_size`, the job it
/// went into; an attached one around the cell of the berth it sits at.
///
/// Returns `false` when no free cell exists within the search radius — the
/// entity stays off the grid, and the caller is expected to retry. Returns
/// `true` for an entity already standing on the grid.
pub(crate) fn place_back_near(
    world: &mut World,
    entity: Entity,
    around: CellPos,
    around_size: CellSize,
) -> bool {
    if entity_def::stands_on_grid(world, entity) {
        return true;
    }
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can be placed back");
    let Some(cell) = cell_back_near(world, entity, around, around_size, &location_def) else {
        return false;
    };
    place_back_at(world, entity, cell, &location_def);
    true
}

/// The cell [`place_back_near`] would stand `entity` on, or `None` when
/// nothing within the search radius takes a footprint of `fits`: an attached
/// entity searches around the cell of the berth it sits at, a hidden one
/// around the footprint at `around`/`around_size`.
pub(crate) fn cell_back_near(
    world: &World,
    entity: Entity,
    around: CellPos,
    around_size: CellSize,
    fits: &LocationDef,
) -> Option<CellPos> {
    let (around, around_size) = if world.entity(entity).contains::<AttachedComponent>() {
        (
            body::anchor(entity_def::position(world, entity)),
            CellSize::new(1, 1),
        )
    } else {
        (around, around_size)
    };
    world
        .resource::<Map>()
        .find_placement_near(around, around_size, fits)
}

/// Like [`place_back_near`], but for callers that cannot retry themselves
/// (order cancellation and other paths that finish in the same tick).
///
/// Places the entity immediately when a cell is free. Otherwise it stays off
/// the grid and is tagged with [`PendingRevealComponent`], so
/// [`game_loop::pending_reveal::process_pending_reveals`] keeps retrying every
/// tick until a cell opens — rather than forcing it onto an occupied cell and
/// corrupting the nav grid.
pub(crate) fn place_back_near_or_retry(
    world: &mut World,
    entity: Entity,
    around: CellPos,
    around_size: CellSize,
) {
    if place_back_near(world, entity, around, around_size) {
        return;
    }

    world.entity_mut(entity).insert(PendingRevealComponent {
        around,
        around_size,
    });
}

/// Stands an attached entity on `cell`, giving its berth up.
///
/// Panics for an entity that is not attached: only a seated body steps out.
pub(crate) fn step_out(world: &mut World, entity: Entity, cell: CellPos) {
    assert!(
        world.entity(entity).contains::<AttachedComponent>(),
        "only an attached entity steps out of a berth"
    );
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef stand");
    place_back_at(world, entity, cell, &location_def);
}

/// Stands an entity that holds no cells and no berth — one that has just given
/// its seat up through [`unseat`] — on the nearest free cell around the
/// footprint at `around`/`around_size`. With no free cell within the search
/// radius it is hidden instead and tagged with [`PendingRevealComponent`], so
/// [`game_loop::pending_reveal::process_pending_reveals`] stands it up as soon
/// as a cell opens.
pub(crate) fn stand_or_retry(
    world: &mut World,
    entity: Entity,
    around: CellPos,
    around_size: CellSize,
) {
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can stand");
    match cell_back_near(world, entity, around, around_size, &location_def) {
        Some(cell) => place_back_at(world, entity, cell, &location_def),
        None => {
            world.entity_mut(entity).insert((
                HiddenComponent,
                PendingRevealComponent {
                    around,
                    around_size,
                },
            ));
        }
    }
}

/// Aligns `breeder`'s brood with the form it has just taken, `origin` being
/// the form the change was declared on. Each of the `seated` broodlings is
/// moved to a seat of the group its attachment names on the new form, nearest
/// its old berth, or **detached** when that form does not hold it — no such
/// group, or no seat left — on the breeder's orphan terms.
pub(crate) fn align_broodlings(
    world: &mut World,
    breeder: Entity,
    seated: &[SimulationId],
    origin: EntityTypeId,
) {
    let Some(fate) = brood::terms_on(world, breeder, origin).map(BreederDef::orphans) else {
        debug_assert!(
            seated.is_empty(),
            "a seated brood was bred on some form's terms"
        );
        return;
    };
    let of = entity_def::simulation_id(world, breeder);
    // Every seat is given up first, so the group is laid out afresh for the
    // form now worn before anyone sits down in it. The tie stays.
    let unseated: Vec<(Entity, CellPos)> = seated
        .iter()
        .map(|&id| {
            let entity = world
                .resource::<EntityIndex>()
                .alive(id)
                .expect("a counted broodling is alive");
            let berth = body::anchor(entity_def::position(world, entity));
            unseat(world, entity).expect("a counted broodling sits in a berth");
            (entity, berth)
        })
        .collect();
    for (entity, berth) in unseated {
        // Its own copy of the attachment: the seating below changes the world.
        let attachment = brood::broodling_attachment(world, entity)
            .cloned()
            .expect("a seated broodling's type is a broodling");
        if brood::holds(world, breeder, entity, &attachment) {
            seat_unseated(world, entity, breeder, &attachment);
        } else {
            detach_broodling(world, breeder, of, entity, berth, fate);
        }
    }
}

/// Seats a bred entity again after a change of form ended early with it back
/// in its bred form: in a free berth of its breeder's group, back on its
/// brood — or, when its breeder is gone or has no seat left for it,
/// **destroyed**. Nothing for an entity nobody bred.
pub(crate) fn reseat_broodling(world: &mut World, entity: Entity) {
    let Some(bred) = world.entity(entity).get::<BredComponent>().cloned() else {
        return;
    };
    let id = entity_def::simulation_id(world, entity);
    let Some(breeder) = world.resource::<EntityIndex>().alive(bred.by) else {
        despawn_entity(world, entity, DeathCause::Unseated { of: bred.by });
        return;
    };
    // Its own copy of the attachment: the seating below changes the world.
    let attachment = brood::broodling_attachment(world, entity)
        .cloned()
        .expect("a bred entity's type is a broodling");
    if brood::holds(world, breeder, entity, &attachment) {
        attach_entity(world, entity, breeder, &attachment);
        brood::add(world, breeder, id);
    } else {
        despawn_entity(world, entity, DeathCause::Unseated { of: bred.by });
    }
}

/// Puts an entity that is off the grid back on it at `cell`, announcing the
/// return of one that was hidden.
///
/// The one point every return to the grid ends at.
fn place_back_at(world: &mut World, entity: Entity, cell: CellPos, location_def: &LocationDef) {
    let class = OccupancyClass::of(entity_def::of(world, entity));
    unseat(world, entity);
    let mut entity_mut = world.entity_mut(entity);

    let was_hidden = entity_mut.contains::<HiddenComponent>();
    entity_mut.remove::<HiddenComponent>();

    let mut location = entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can be placed back");
    location.position = FixedUVec2::from(cell);

    let location = *location;
    world
        .resource_mut::<Map>()
        .place_entity(&location, location_def, class);

    if was_hidden {
        let announced = SimulationEvent::EntityRevealed {
            entity: entity_def::simulation_id(world, entity),
        };
        world.resource_mut::<EventRecord>().emit(announced);
    }
}

/// Takes `source` off the map under the `site` raised over it, handing the
/// site what the source had left: the source's footprint is already the
/// site's, so it leaves without freeing anything, and the site remembers what
/// to put back. The source dies as overbuilt. [`uncover_source`] is the other
/// half.
pub(crate) fn cover_source(world: &mut World, source: Entity, site: Entity) {
    let amount = world
        .entity(source)
        .get::<ResourceSourceComponent>()
        .expect("a source under a site is a resource source")
        .amount;
    let uncovers = entity_def::type_id(world, source);
    let anchor = body::anchor(entity_def::position(world, source));
    world.entity_mut(source).insert(HiddenComponent);
    despawn_entity(world, source, DeathCause::Overbuilt);

    let mut site_mut = world.entity_mut(site);
    site_mut
        .get_mut::<ResourceSourceComponent>()
        .expect("a type raised over a source is a resource source")
        .amount = amount;
    site_mut.insert(OverbuiltComponent { uncovers, anchor });
}

/// Puts back the resource source `entity` was raised over, on the cells it was
/// covered on and with what it had left, announcing the return; nothing for an
/// entity raised on open ground. One drained dry comes back empty or not at
/// all, as the source type's own depletion says.
///
/// A caller uncovers once the entity's own footprint is off the grid. When
/// those cells are no longer free even so — something walked onto ground the
/// cover never claimed — the source does not come back at all, the way blocked
/// ground leaves no remains (see [`spawn_corpse_entity`]).
///
/// A cover that wears a form which is no resource source holds nothing, so what
/// it gives back is an empty source or none at all, as if it had been drained.
pub(crate) fn uncover_source(world: &mut World, entity: Entity) {
    let Some(overbuilt) = world.entity(entity).get::<OverbuiltComponent>().copied() else {
        return;
    };
    let amount = world
        .entity(entity)
        .get::<ResourceSourceComponent>()
        .map_or(0, |source| source.amount);
    let (type_name, depletion) = {
        let def = world.resource::<ContentRegistry>().def(overbuilt.uncovers);
        (
            def.name.clone(),
            def.resource_source
                .as_ref()
                .expect("an overbuilt type is a resource source")
                .depletion(),
        )
    };
    if amount == 0 {
        match depletion {
            DepletionPolicy::Persist => {}
            DepletionPolicy::Destroy => return,
        }
    }
    let by = entity_def::simulation_id(world, entity);
    if let Some((source, _)) = spawn_entity(
        world,
        &type_name,
        FixedUVec2::from(overbuilt.anchor),
        None,
        SpawnCause::Uncovered { by },
        FieldReach::Full,
    ) {
        world
            .entity_mut(source)
            .get_mut::<ResourceSourceComponent>()
            .expect("an overbuilt type is a resource source")
            .amount = amount;
    }
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
/// Announces nothing; [`despawn_entity`] is the announcing counterpart, as
/// [`spawn_entity`] is to [`create_entity`].
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
    // claim already tracks the cell they round to.
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => {
            if movement_model::is_mid_crossing(location.position)
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
    settle_broodlings(world, entity, entity_def::morph_origin(world, entity));
    leave_brood(world, entity, id);

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

/// Starts the dying phase for an alive entity and announces the death.
///
/// The announcing counterpart to [`destroy_entity`]. Announced *before* the
/// teardown runs, while the entity still carries what the announcement reports;
/// anything aboard dies during that teardown, so the holder's death precedes its
/// passengers' in the record.
///
/// No-op if the entity is already dying or has died, so a death is never
/// announced twice.
pub fn despawn_entity(world: &mut World, entity: Entity, cause: DeathCause) {
    {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<DyingComponent>() || entity_ref.contains::<DiedComponent>() {
            return;
        }
    }

    let announced = SimulationEvent::EntityDied {
        entity: entity_def::simulation_id(world, entity),
        entity_type: entity_def::type_id(world, entity),
        owner: entity_def::owner(world, entity),
        position: entity_def::position(world, entity),
        cause,
    };
    world.resource_mut::<EventRecord>().emit(announced);

    destroy_entity(world, entity);
}

/// Hands `entity` to `to`, announcing the capture, with `by` naming what took
/// it.
///
/// The entity keeps everything it holds: its health, its pools, and the orders
/// in its queue, which carry on for their new owner. It leaves every player's
/// selection and control groups, the two stores that hold ids.
///
/// Panics if `to` already owns it.
pub fn change_owner(world: &mut World, entity: Entity, to: PlayerId, by: SimulationId) {
    let from = entity_def::owner(world, entity);
    assert!(
        from != Some(to),
        "an entity is handed only to a player that does not already own it"
    );

    let id = entity_def::simulation_id(world, entity);
    world.resource_mut::<Selection>().remove(id);
    world.resource_mut::<ControlGroups>().remove(id);
    world.entity_mut(entity).insert(OwnerComponent::new(to));

    let announced = SimulationEvent::EntityCaptured {
        entity: id,
        entity_type: entity_def::type_id(world, entity),
        from,
        to,
        by,
        position: entity_def::position(world, entity),
    };
    world.resource_mut::<EventRecord>().emit(announced);
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
    let around = entity_def::footprint_rect(world, entity);
    let (around, around_size) = (around.origin, around.size);
    let holder = entity_def::simulation_id(world, entity);

    for id in passengers {
        let Some(passenger) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        world
            .entity_mut(passenger)
            .remove::<(BoardedComponent, GarrisonFireComponent)>();
        match fate {
            PassengerFate::Destroy => {
                despawn_entity(world, passenger, DeathCause::PassengerLost { holder })
            }
            PassengerFate::Eject => {
                if !place_back_near(world, passenger, around, around_size) {
                    despawn_entity(world, passenger, DeathCause::PassengerLost { holder });
                }
            }
        }
    }
}

/// Settles every broodling `breeder` counts on the orphan terms it breeds on
/// with `origin` as the form its change was declared on, taking them off its
/// brood: perishing ones die in id order, lingering ones are set down by the
/// berths they sat at, the tie cut either way. Nothing for a breeder that
/// breeds nothing on either form.
pub(crate) fn settle_broodlings(world: &mut World, breeder: Entity, origin: EntityTypeId) {
    let Some(fate) = brood::terms_on(world, breeder, origin).map(BreederDef::orphans) else {
        return;
    };
    let of = entity_def::simulation_id(world, breeder);
    let Some(broodlings) = world
        .entity_mut(breeder)
        .get_mut::<BroodComponent>()
        .map(|mut brood| std::mem::take(&mut brood.broodlings))
    else {
        return;
    };

    for id in broodlings {
        let broodling = world
            .resource::<EntityIndex>()
            .alive(id)
            .expect("a counted broodling is alive");
        world.entity_mut(broodling).remove::<BredComponent>();
        match fate {
            OrphanFate::Perish => despawn_entity(world, broodling, DeathCause::Orphaned { of }),
            OrphanFate::Linger(_) => {
                // Set down by the berth it sat at, where it was seen.
                let berth = body::anchor(entity_def::position(world, broodling));
                let size = entity_def::footprint(world, broodling).1;
                unseat(world, broodling);
                stand_or_retry(world, broodling, berth, size);
            }
        }
    }
}

/// Detaches a broodling that has given its seat up and that the form
/// `breeder` wears does not hold, on `fate`'s terms: perishing, it dies
/// **unseated**; lingering, its tie is cut and it is set down by `berth`, the
/// cell it sat at.
fn detach_broodling(
    world: &mut World,
    breeder: Entity,
    of: SimulationId,
    entity: Entity,
    berth: CellPos,
    fate: OrphanFate,
) {
    match fate {
        OrphanFate::Perish => despawn_entity(world, entity, DeathCause::Unseated { of }),
        OrphanFate::Linger(_) => {
            brood::untie(world, breeder, entity);
            let size = entity_def::footprint(world, entity).1;
            stand_or_retry(world, entity, berth, size);
        }
    }
}

/// Takes a dying broodling off its breeder's count.
fn leave_brood(world: &mut World, entity: Entity, id: SimulationId) {
    let Some(bred) = world.entity_mut(entity).take::<BredComponent>() else {
        return;
    };
    if let Some(breeder) = world.resource::<EntityIndex>().alive(bred.by) {
        brood::remove(world, breeder, id);
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
/// only an entity that has none is given its type's default. Field sources
/// reach as far as `reach` says, the acts on standing are armed or kept as
/// `acts` says, and what a change of form carries through an interim form —
/// a brood, a rally point — stays or goes as `wearing` says.
pub(crate) fn fit_components(
    world: &mut World,
    entity: Entity,
    type_id: EntityTypeId,
    reach: FieldReach,
    acts: StandingActs,
    wearing: Wearing,
) {
    let (
        can_attack,
        mounted_turrets,
        can_move,
        has_health,
        trainer,
        transporter,
        source,
        carrier,
        annex,
        docks,
        tags,
        skills,
        field_sources,
        acts_on_standing,
        breeds,
        breeds_producers,
    ) = {
        let registry = world.resource::<ContentRegistry>();
        let def = registry.def(type_id);
        (
            def.can_attack(),
            def.turrets.len(),
            def.can_move(),
            def.has_health(),
            def.trainer.is_some(),
            def.can_transport(),
            def.resource_source.is_some(),
            def.resource_carrier.is_some(),
            def.annex.is_some(),
            !def.docks.is_empty(),
            def.tags.clone(),
            def.skills.clone(),
            def.field_sources.clone(),
            !def.on_stand.is_empty(),
            def.breeder.is_some(),
            def.breeder.as_ref().is_some_and(|breeder| {
                registry
                    .entity(breeder.breeds())
                    .expect("validated content breeds registered types")
                    .produces_by_morph()
            }),
        )
    };
    // A rally point serves whatever releases units: the trainer, the holder,
    // the breeder whose broodlings make units by a change of form, and the
    // form making one that way now.
    let producing = match wearing {
        Wearing::Interim => world
            .entity(entity)
            .get::<MorphComponent>()
            .and_then(|morph| entity_def::morph_terms(world, morph))
            .is_some_and(|transition| match transition.reason() {
                MorphReason::Production => true,
                MorphReason::Change => false,
            }),
        Wearing::Own => false,
    };
    let wants_rally = trainer || transporter || breeds_producers || producing;
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
    fit_default::<ResourceSourceComponent>(&mut entity_mut, source);
    fit_default::<ResourceCarrierComponent>(&mut entity_mut, carrier);
    // Which primary an annex stands with is re-derived every tick, so a form
    // that becomes an annex starts alone and is docked by the next pass.
    // Standing alone is a state with terms of its own, not an empty one, so it
    // is fitted by name rather than by default.
    match (annex, entity_mut.contains::<AnnexComponent>()) {
        (true, false) => {
            entity_mut.insert(AnnexComponent::alone());
        }
        (false, true) => {
            entity_mut.remove::<AnnexComponent>();
        }
        (true, true) | (false, false) => {}
    }
    // A rally point is live state — where the player pointed — so it stays
    // through a form change, including one wearing an interim form that
    // releases nothing on the way, and goes only with a form that releases
    // nothing for good.
    match (wants_rally, entity_mut.contains::<RallyPointComponent>()) {
        (true, false) => {
            entity_mut.insert(RallyPointComponent::default());
        }
        (false, true) => match wearing {
            Wearing::Interim => {}
            Wearing::Own => {
                entity_mut.remove::<RallyPointComponent>();
            }
        },
        (true, true) | (false, false) => {}
    }
    // A brood is live state — the broodlings counted, the ticks toward the
    // next — so it stays through a form change, including one wearing an
    // interim form that breeds nothing on the way, and goes only with a form
    // that breeds nothing for good.
    match (breeds, entity_mut.contains::<BroodComponent>()) {
        (true, false) => {
            entity_mut.insert(BroodComponent::default());
        }
        (false, true) => match wearing {
            Wearing::Interim => {}
            Wearing::Own => {
                entity_mut.remove::<BroodComponent>();
            }
        },
        (true, true) | (false, false) => {}
    }
    // Offering docks is a standing fact about a type, so the component that
    // records what stands in them is fitted with the form rather than by
    // whatever settles the bonds: a primary with every dock empty still has
    // one, which is what makes "offers docks" a query rather than a lookup.
    // A form that stops offering docks drops what stood in them there and then.
    fit_default::<DocksComponent>(&mut entity_mut, docks);

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

    if acts_on_standing {
        match acts {
            StandingActs::Rearm => {
                entity_mut.insert(StandComponent(Standing::Pending));
            }
            StandingActs::Keep => {
                if !entity_mut.contains::<StandComponent>() {
                    entity_mut.insert(StandComponent(Standing::Pending));
                }
            }
        }
    } else {
        entity_mut.remove::<StandComponent>();
    }

    // Field sources reach as far as asked. Left to grow, a source takes over
    // the reach of the one it replaces at the same index, so a form change
    // does not pull a field back to its seed.
    if field_sources.is_empty() {
        entity_mut.remove::<FieldSourcesComponent>();
    } else {
        let sources = match reach {
            FieldReach::Full => FieldSourcesComponent::full(&field_sources),
            FieldReach::Initial => match entity_mut.get::<FieldSourcesComponent>() {
                Some(previous) => FieldSourcesComponent::carried(&field_sources, previous),
                None => FieldSourcesComponent::seeded(&field_sources),
            },
        };
        entity_mut.insert(sources);
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
