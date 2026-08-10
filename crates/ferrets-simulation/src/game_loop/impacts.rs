//! Delivery of a weapon's damage: immediately, or as a shot that lands later.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection},
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::layer_mask::LayerMask;

use super::damage;
use crate::{
    components::{
        dying::DyingComponent, entity_info::EntityInfoComponent, health::HealthComponent,
        hidden::HiddenComponent, location::LocationComponent, owner::OwnerComponent,
    },
    content::{
        entity_type_def::EntityTypeDef,
        projectile::Aim,
        registry::ContentRegistry,
        splash::{SplashDef, SplashShape},
    },
    entity_def,
    entity_index::EntityIndex,
    impacts::{PendingImpact, PendingImpacts},
    map::Map,
    session::GameSession,
    simulation_id::SimulationId,
};

/// Delivers one hit from `attacker` against `target`, released from `origin`
/// — the attacker's own position, or its holder's for a weapon fired from
/// inside something else, whose bearer stands nowhere.
///
/// A type with a projectile releases a shot that lands after its flight time; one
/// without applies the damage now. A target-following shot damages `target` wherever
/// it has moved to and centres its blast there; a cell-aimed one commits to the cell
/// `target` occupied at release, so a target that keeps moving escapes it.
pub(super) fn deliver(
    world: &mut World,
    attacker: Entity,
    origin: FixedUVec2,
    target: Option<Entity>,
    aimed_at: FixedUVec2,
    damage: FixedU64,
) {
    let Some(attacker_id) = world
        .entity(attacker)
        .get::<EntityInfoComponent>()
        .map(EntityInfoComponent::id)
    else {
        return;
    };
    let target_id = target.and_then(|target| target_id(world, target));
    let impact = aimed_at;

    let projectile = entity_def::of(world, attacker).projectile;
    match projectile {
        None => {
            let def = entity_def::of(world, attacker).clone();
            land(world, &def, attacker_id, target_id, impact, origin, damage);
        }
        Some(projectile) => {
            let (speed, aim) = {
                let def = world
                    .resource::<ContentRegistry>()
                    .projectile_def(projectile);
                (def.speed(), def.aim())
            };
            // Footprint-based like every range check, so the flight covers the same
            // distance acquisition measured — a shot at a 3x3 building is not treated
            // as reaching for its origin corner.
            let target_size = target
                .and_then(|target| entity_def::of(world, target).location)
                .map(|location| location.size())
                .unwrap_or(CellSize::ONE);
            let distance = world.resource::<Map>().projection().rect_distance(
                CellPos::from(origin),
                CellRect::new(CellPos::from(impact), target_size),
            );
            let tick = world.resource::<GameSession>().tick();
            let attacker_type = world
                .entity(attacker)
                .get::<EntityInfoComponent>()
                .expect("attacker carries entity info")
                .type_id();
            let shot = PendingImpact {
                attacker: attacker_id,
                attacker_type,
                projectile,
                // A cell-aimed shot has nobody to follow once it is in the air.
                target: match aim {
                    Aim::Entity => target_id,
                    Aim::Position => None,
                },
                origin,
                impact,
                damage,
                emitted_on_tick: tick,
                lands_on_tick: tick + flight_ticks(FixedU64::from_num(distance), speed),
            };
            world.resource_mut::<PendingImpacts>().push(shot);
        }
    }
}

/// Lands every shot due this tick, in release order.
///
/// Runs where the same-tick delivery path applies its damage, so a shot and a
/// point-blank hit reach their victims at the same point in the tick.
pub fn process_impacts(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();
    let due = world.resource_mut::<PendingImpacts>().take_due(tick);
    for shot in due {
        // The firing type outlives the firing entity, so the shot still deals its
        // bonuses even when the attacker is gone.
        let def = world
            .resource::<ContentRegistry>()
            .def(shot.attacker_type)
            .clone();
        // A shot that follows its target resolves where the target is now, so its
        // blast lands with it; one aimed at a cell resolves where it was sent.
        let target = shot
            .target
            .and_then(|id| world.resource::<EntityIndex>().alive(id));
        let impact = target.map_or(shot.impact, |target| position_of(world, target));
        land(
            world,
            &def,
            shot.attacker,
            shot.target,
            impact,
            shot.origin,
            shot.damage,
        );
    }
}

/// Applies a hit's direct damage and its blast, if the firing type has one.
///
/// The firing type arrives already resolved because the two callers find it
/// differently: a same-tick hit reads it off the attacker, while a landing shot
/// reads it from the registry — the attacker may be dead by then.
fn land(
    world: &mut World,
    attacker_def: &EntityTypeDef,
    attacker_id: SimulationId,
    target_id: Option<SimulationId>,
    impact: FixedUVec2,
    origin: FixedUVec2,
    damage: FixedU64,
) {
    // Who takes the hit at full strength: the entity the shot followed, wherever it
    // moved to, or — for a shot sent to a cell — whoever is standing on that cell when
    // it arrives. Either way, an empty answer means the shot was wasted.
    let direct: Vec<Entity> = match target_id {
        Some(id) => world
            .resource::<EntityIndex>()
            .alive(id)
            .into_iter()
            .collect(),
        None => occupants_of(world, impact),
    };
    for &victim in &direct {
        let dealt = damage::resolve(world, attacker_def, victim, damage);
        damage::apply(world, attacker_id, victim, dealt);
    }

    let Some(splash) = attacker_def.splash.as_ref() else {
        return;
    };
    let victims = blast_victims(world, splash, attacker_id, &direct, impact, origin);
    for (victim, fraction) in victims {
        let dealt = damage::resolve_scaled(world, attacker_def, victim, damage, fraction);
        damage::apply(world, attacker_id, victim, dealt);
    }
}

/// The entities a blast catches, each with the fraction of the hit's damage its
/// band deals, in [`SimulationId`] order.
///
/// The order is fixed so two peers kill the same victims in the same sequence.
fn blast_victims(
    world: &mut World,
    splash: &SplashDef,
    attacker_id: SimulationId,
    direct: &[Entity],
    impact: FixedUVec2,
    origin: FixedUVec2,
) -> Vec<(Entity, FixedU64)> {
    // Gather first, then filter: the checks below read the world immutably.
    let mut query = world.query::<(Entity, &EntityInfoComponent, &LocationComponent)>();
    let candidates: Vec<(Entity, SimulationId, FixedUVec2)> = query
        .iter(world)
        .map(|(entity, info, location)| (entity, info.id(), location.position))
        .collect();

    let projection = world.resource::<Map>().projection();
    let session = world.resource::<GameSession>();
    let attacker_owner = world
        .resource::<EntityIndex>()
        .alive(attacker_id)
        .and_then(|attacker| world.entity(attacker).get::<OwnerComponent>().copied());

    let mut caught: Vec<(SimulationId, Entity, FixedU64)> = Vec::new();
    for (entity, id, position) in candidates {
        if direct.contains(&entity) {
            continue;
        }
        let entity_ref = world.entity(entity);
        // Only damageable entities that are not already dying. Hidden entities
        // hold no cells and their position is stale, so no blast reaches them.
        if entity_ref.get::<HealthComponent>().is_none()
            || entity_ref.contains::<DyingComponent>()
            || entity_ref.contains::<HiddenComponent>()
        {
            continue;
        }
        // The blast only reaches the layers content gave it.
        let occupation = entity_def::of(world, entity)
            .location
            .map(|location| location.occupation())
            .unwrap_or(LayerMask::EMPTY);
        if occupation & splash.layers() == LayerMask::EMPTY {
            continue;
        }
        // Without friendly fire the blast spares the attacker's own side. Neutrals
        // are nobody's ally, so they are still caught.
        if !splash.friendly_fire()
            && let (Some(attacker_owner), Some(victim_owner)) =
                (attacker_owner, entity_ref.get::<OwnerComponent>())
            && session.are_allied(attacker_owner.player(), victim_owner.player())
        {
            continue;
        }
        let Some(fraction) = band_fraction(splash, projection, origin, impact, position) else {
            continue;
        };
        caught.push((id, entity, fraction));
    }

    caught.sort_by_key(|&(id, ..)| id);
    caught
        .into_iter()
        .map(|(_, entity, fraction)| (entity, fraction))
        .collect()
}

/// The fraction the innermost band containing `victim` deals, or `None` when the
/// blast does not reach it.
fn band_fraction(
    splash: &SplashDef,
    projection: Projection,
    origin: FixedUVec2,
    impact: FixedUVec2,
    victim: FixedUVec2,
) -> Option<FixedU64> {
    let victim = CellPos::from(victim);
    splash
        .bands()
        .find(|&(radius, _)| match splash.shape() {
            SplashShape::Circular => projection.in_range(CellPos::from(impact), victim, radius),
            SplashShape::Line => near_path(projection, origin, impact, victim, radius),
        })
        .map(|(_, fraction)| fraction)
}

/// Whether `victim` lies within `radius` of the shot's path, sampled at one-cell
/// steps from `origin` to `impact`.
fn near_path(
    projection: Projection,
    origin: FixedUVec2,
    impact: FixedUVec2,
    victim: CellPos,
    radius: u32,
) -> bool {
    let from = CellPos::from(origin);
    let to = CellPos::from(impact);
    // Chebyshev here is sampling density — the number of one-cell steps an
    // 8-connected walk of the segment takes — not a range metric; the range
    // check below is the projection's.
    let steps = projection::chebyshev(from, to);
    for step in 0..=steps {
        let sample = lerp_cell(from, to, step, steps);
        if projection.in_range(sample, victim, radius) {
            return true;
        }
    }
    false
}

/// The cell `step` of `steps` along the way from `from` to `to`.
fn lerp_cell(from: CellPos, to: CellPos, step: u32, steps: u32) -> CellPos {
    if steps == 0 {
        return from;
    }
    let along = |a: u32, b: u32| -> u32 {
        let delta = i64::from(b) - i64::from(a);
        let moved = delta * i64::from(step) / i64::from(steps);
        u32::try_from(i64::from(a) + moved).unwrap_or(a)
    };
    CellPos::new(along(from.x, to.x), along(from.y, to.y))
}

/// The damageable entities whose footprint covers `cell`, in [`SimulationId`] order.
///
/// More than one can share a cell when their footprints occupy layers that do not
/// collide, so this answers with all of them rather than picking one.
fn occupants_of(world: &mut World, cell: FixedUVec2) -> Vec<Entity> {
    let cell = CellPos::from(cell);
    let mut query = world.query::<(Entity, &EntityInfoComponent, &LocationComponent)>();
    let candidates: Vec<(Entity, SimulationId, CellPos)> = query
        .iter(world)
        .map(|(entity, info, location)| (entity, info.id(), CellPos::from(location.position)))
        .collect();

    let mut found: Vec<(SimulationId, Entity)> = Vec::new();
    for (entity, id, origin) in candidates {
        let entity_ref = world.entity(entity);
        // Hidden entities hold no cells and their position is stale, so a shot
        // arriving at the cell one stood on finds nobody there.
        if entity_ref.get::<HealthComponent>().is_none()
            || entity_ref.contains::<DyingComponent>()
            || entity_ref.contains::<HiddenComponent>()
        {
            continue;
        }
        let Some(size) = entity_def::of(world, entity)
            .location
            .map(|location| location.size())
        else {
            continue;
        };
        if cell.x >= origin.x
            && cell.x < origin.x + size.width
            && cell.y >= origin.y
            && cell.y < origin.y + size.height
        {
            found.push((id, entity));
        }
    }

    found.sort_by_key(|&(id, _)| id);
    found.into_iter().map(|(_, entity)| entity).collect()
}

/// The whole ticks a flight of `distance` at `speed` takes, at least one so a shot
/// never lands on the tick it was released.
fn flight_ticks(distance: FixedU64, speed: FixedU64) -> u32 {
    (distance / speed).ceil().to_num::<u32>().max(1)
}

/// The entity's current position.
fn position_of(world: &World, entity: Entity) -> FixedUVec2 {
    world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.position)
        .unwrap_or_default()
}

/// The entity's simulation id, if it carries one.
fn target_id(world: &World, entity: Entity) -> Option<SimulationId> {
    world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .map(EntityInfoComponent::id)
}
