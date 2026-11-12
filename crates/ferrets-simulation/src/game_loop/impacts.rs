//! Delivery of a weapon's damage: immediately, or as a shot that lands later.

use bevy_ecs::{entity::Entity, query::With, world::World};
use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    projection::{self, Projection},
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::layer_mask::LayerMask;

use super::{buffs, damage};
use crate::{
    components::{
        dying::DyingComponent, entity_info::EntityInfoComponent, hidden::HiddenComponent,
        location::LocationComponent, owner::OwnerComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    impacts::{AimedAt, FiredFrom, Following, PendingImpact, PendingImpacts},
    map::Map,
    session::GameSession,
    simulation_id::SimulationId,
};
use ferrets_content::{
    area::ResolvedArea,
    attack::{Delivery, Slain, Weapon},
    entity_buffs::Interruption,
    entity_type_def::EntityTypeDef,
    pool_def::PoolId,
    projectile::Aim,
    registry::ContentRegistry,
    splash::{SplashDef, SplashShape},
    targeting,
};

/// Delivers one hit from `attacker`'s `fired_from` weapon against what `aimed`
/// names — a body, or a point on the ground — released from `origin`, where
/// that weapon sits: the middle of the body for the weapon it points itself,
/// the middle of a turret's mount for a turret, and the middle of the holder
/// for a weapon fired from inside something else, whose bearer stands nowhere.
///
/// A type with a projectile releases a shot that lands after its flight time;
/// one without applies the damage now. A shot following a body damages it
/// wherever it has moved to and centers its blast there; a cell-aimed one
/// commits to the cell the body stood on nearest the weapon at release, so a
/// body that keeps moving escapes it; a shot at a bare cell lands at its
/// middle.
///
/// Firing is the attacker's attack: every buff of its that an attack cuts
/// short ends here, whatever weapon fired and whether or not the shot then
/// finds a victim.
pub(super) fn deliver(
    world: &mut World,
    attacker: Entity,
    fired_from: FiredFrom,
    origin: FixedUVec2,
    aimed: AimedAt,
    damage: FixedU64,
) {
    let Some(attacker_id) = world
        .entity(attacker)
        .get::<EntityInfoComponent>()
        .map(EntityInfoComponent::id)
    else {
        return;
    };
    let target = aimed.body();
    let target_id = target.and_then(|target| target_id(world, target));
    // A shot at an entity is aimed at the middle of the cell it stands on
    // nearest the weapon; a shot at bare ground at the middle of the cell it
    // was sent to.
    let impact = match aimed {
        AimedAt::Body(target) => aim_cell(world, origin, target),
        AimedAt::Cell(cell) => cell.middle(),
    };

    let delivery = fired_from
        .weapon(
            world.resource::<ContentRegistry>(),
            entity_def::of(world, attacker),
        )
        .map(Weapon::delivery)
        .expect("a delivering entity has the weapon it fired");

    let blast = blast_bands(world, attacker, fired_from);

    buffs::interrupt_entity_buffs(world, attacker, Interruption::Attack);

    match delivery {
        Delivery::Instant => {
            let def = entity_def::of(world, attacker).clone();
            land(
                world,
                &def,
                fired_from,
                attacker_id,
                target_id.map_or(Following::Nothing, Following::Target),
                impact,
                origin,
                damage,
                blast.as_deref(),
            );
        }
        Delivery::Projectile(projectile) => {
            let (speed, aim) = {
                let def = world
                    .resource::<ContentRegistry>()
                    .projectile_def(projectile);
                (def.speed(), def.aim())
            };
            // The flight is the path from where the shot leaves to where it
            // lands.
            let distance = world
                .resource::<Map>()
                .projection()
                .span(origin.x.abs_diff(impact.x), origin.y.abs_diff(impact.y));
            let tick = world.resource::<GameSession>().tick();
            let attacker_type = entity_def::type_id(world, attacker);
            let shot = PendingImpact {
                attacker: attacker_id,
                attacker_type,
                fired_from,
                projectile,
                // A cell-aimed shot has nobody to follow once it is in the air.
                target: match (aim, target_id) {
                    (Aim::Entity, Some(id)) => Following::Target(id),
                    (Aim::Entity, None) | (Aim::Position, _) => Following::Nothing,
                },
                origin,
                impact,
                damage,
                blast,
                emitted_on_tick: tick,
                lands_on_tick: tick + flight_ticks(distance, speed),
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
        // A shot that follows its target resolves where the target stands or
        // lies now, so its blast lands with it; one aimed at a cell, one whose
        // target went off the map (sent where it went in), or one whose target
        // is gone resolves where it was sent.
        let impact = match shot.target {
            // A target hidden while this batch lands, after the latch took
            // the batch out of the queue, is lost as the latch would lose it.
            Following::Target(id) => match world.resource::<EntityIndex>().any(id) {
                Some(target) => match world.get::<HiddenComponent>(target) {
                    Some(hidden) => hidden.went_in.position,
                    None => aim_cell(world, shot.origin, target),
                },
                None => shot.impact,
            },
            Following::Lost | Following::Nothing => shot.impact,
        };
        land(
            world,
            &def,
            shot.fired_from,
            shot.attacker,
            shot.target,
            impact,
            shot.origin,
            shot.damage,
            shot.blast.as_deref(),
        );
    }
}

/// Applies a hit's direct damage and its blast, if the firing type has one.
///
/// The firing type arrives already resolved because the two callers find it
/// differently: a same-tick hit reads it off the attacker, while a landing shot
/// reads it from the registry — the attacker may be dead by then.
#[allow(clippy::too_many_arguments)]
fn land(
    world: &mut World,
    attacker_def: &EntityTypeDef,
    fired_from: FiredFrom,
    attacker_id: SimulationId,
    target: Following,
    impact: FixedUVec2,
    origin: FixedUVec2,
    damage: FixedU64,
    blast: Option<&[(ResolvedArea, FixedU64)]>,
) {
    // Who takes the hit at full strength: the entity the shot followed, wherever it
    // moved to, and nobody once it is gone or lost; a shot sent to a cell hits
    // whoever is standing on it when it arrives. An empty answer means the shot was
    // wasted, its blast aside.
    let direct: Vec<Entity> = match target {
        Following::Target(id) => world
            .resource::<EntityIndex>()
            .interactable(world, id)
            .into_iter()
            .collect(),
        Following::Lost => Vec::new(),
        Following::Nothing => occupants_of(world, impact),
    };
    // A weapon still cannot reach layers it does not target, even standing on
    // the cell the shot arrived at: a shell sent to the ground passes under
    // whatever drifted overhead while it was in flight.
    let weapon = fired_from
        .weapon(world.resource::<ContentRegistry>(), attacker_def)
        .cloned();
    let targets = weapon
        .as_ref()
        .map_or(LayerMask::EMPTY, |weapon| weapon.targets());
    let direct: Vec<Entity> = direct
        .into_iter()
        .filter(|&victim| {
            targeting::reaches(
                world.resource::<ContentRegistry>(),
                targets,
                entity_def::of(world, victim),
            )
        })
        .collect();
    // What the weapon leaves of what it kills. A shot whose weapon can no
    // longer be resolved is one nothing reached anyway: the loop below runs
    // over an empty list.
    let slain = weapon.as_ref().map_or(Slain::Remains, Weapon::slain);
    for &victim in &direct {
        let dealt = damage::resolve(world, attacker_def, victim, damage);
        damage::apply(world, attacker_id, victim, dealt, slain);
    }

    let Some(bands) = blast else {
        return;
    };
    let splash = weapon
        .as_ref()
        .and_then(Weapon::splash)
        .expect("a shot frozen with bands was fired by a weapon that splashes");
    let victims = blast_victims(world, splash, bands, attacker_id, &direct, impact, origin);
    for (victim, fraction) in victims {
        let dealt = damage::resolve_scaled(world, attacker_def, victim, damage, fraction);
        damage::apply(world, attacker_id, victim, dealt, slain);
    }
}

/// The entities a blast catches, each with the fraction of full strength its
/// band's hit lands at, in [`SimulationId`] order.
///
/// The order is fixed so two peers kill the same victims in the same sequence.
fn blast_victims(
    world: &mut World,
    splash: &SplashDef,
    blast: &[(ResolvedArea, FixedU64)],
    attacker_id: SimulationId,
    direct: &[Entity],
    impact: FixedUVec2,
    origin: FixedUVec2,
) -> Vec<(Entity, FixedU64)> {
    // Gather first, then filter: the checks below read the world immutably.
    let mut query =
        world.query_filtered::<(Entity, &EntityInfoComponent), With<LocationComponent>>();
    let candidates: Vec<(Entity, SimulationId)> = query
        .iter(world)
        .map(|(entity, info)| (entity, info.id()))
        .collect();

    let projection = world.resource::<Map>().projection();
    let session = world.resource::<GameSession>();
    let attacker_owner = world
        .resource::<EntityIndex>()
        .alive(attacker_id)
        .and_then(|attacker| world.entity(attacker).get::<OwnerComponent>().copied());

    let mut caught: Vec<(SimulationId, Entity, FixedU64)> = Vec::new();
    for (entity, id) in candidates {
        if direct.contains(&entity) {
            continue;
        }
        let entity_ref = world.entity(entity);
        // Only damageable entities that are not already dying; a hidden one has
        // no position for the query to find.
        if !entity_def::has_pool(world, entity, PoolId::HEALTH)
            || entity_ref.contains::<DyingComponent>()
        {
            continue;
        }
        // The blast only reaches the layers content gave it, met where the
        // victim can be attacked; a type with no location stands nowhere.
        let def = entity_def::of(world, entity);
        if def.location.is_none() {
            continue;
        }
        if targeting::targetable(world.resource::<ContentRegistry>(), def) & splash.layers()
            == LayerMask::EMPTY
        {
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
        let Some(fraction) = band_fraction(
            splash.shape(),
            blast,
            projection,
            origin,
            impact,
            entity_def::location(world, entity).footprint(),
        ) else {
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

/// The fraction of full strength of the innermost band of `blast` containing
/// `victim`, a footprint, or `None` when the blast does not reach it: the
/// whole footprint is the victim, so a blast overlapping any of its cells
/// catches it, in the band that cell falls in.
fn band_fraction(
    shape: SplashShape,
    blast: &[(ResolvedArea, FixedU64)],
    projection: Projection,
    origin: FixedUVec2,
    impact: FixedUVec2,
    victim: CellRect,
) -> Option<FixedU64> {
    blast
        .iter()
        .find(|&&(area, _)| {
            let reach = area.reach();
            match shape {
                SplashShape::Circular => {
                    projection.in_range_of_rect(CellPos::from(impact), victim, reach)
                }
                SplashShape::Line => near_path(projection, origin, impact, victim, reach),
            }
        })
        .map(|&(_, fraction)| fraction)
}

/// The bands of the splash of `attacker`'s `fired_from` weapon, as the
/// attacker's stats read now, innermost first: how far each reaches, and the
/// fraction of full strength its hits land at. `None` for a weapon that
/// splashes nothing.
fn blast_bands(
    world: &World,
    attacker: Entity,
    fired_from: FiredFrom,
) -> Option<Vec<(ResolvedArea, FixedU64)>> {
    let splash = fired_from
        .weapon(
            world.resource::<ContentRegistry>(),
            entity_def::of(world, attacker),
        )
        .and_then(Weapon::splash)?;
    let (core, fraction) = splash.core();
    let mut reach = entity_def::resolve_area(world, attacker, core);
    let mut bands = vec![(reach, fraction)];
    for (width, fraction) in splash.rings() {
        reach = reach.widened(entity_def::quantity(world, attacker, width));
        bands.push((reach, fraction));
    }
    Some(bands)
}

/// Whether `victim` lies within `reach` of the shot's path, sampled at one-cell
/// steps from `origin` to `impact`.
fn near_path(
    projection: Projection,
    origin: FixedUVec2,
    impact: FixedUVec2,
    victim: CellRect,
    reach: u32,
) -> bool {
    let from = CellPos::from(origin);
    let to = CellPos::from(impact);
    // Chebyshev here is sampling density — the number of one-cell steps an
    // 8-connected walk of the segment takes — not a range metric; the range
    // check below is the projection's.
    let steps = projection::chebyshev(from, to);
    for step in 0..=steps {
        let sample = lerp_cell(from, to, step, steps);
        if projection.in_range_of_rect(sample, victim, reach) {
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
fn occupants_of(world: &mut World, point: FixedUVec2) -> Vec<Entity> {
    let cell = CellPos::from(point);
    let mut query =
        world.query_filtered::<(Entity, &EntityInfoComponent), With<LocationComponent>>();
    let candidates: Vec<(Entity, SimulationId)> = query
        .iter(world)
        .map(|(entity, info)| (entity, info.id()))
        .collect();

    let mut found: Vec<(SimulationId, Entity)> = Vec::new();
    for (entity, id) in candidates {
        let entity_ref = world.entity(entity);
        // A hidden entity has no position for the query to find: a shot arriving
        // at the cell one stood on finds nobody there.
        if !entity_def::has_pool(world, entity, PoolId::HEALTH)
            || entity_ref.contains::<DyingComponent>()
        {
            continue;
        }
        if entity_def::of(world, entity).location.is_none() {
            continue;
        }
        if entity_def::location(world, entity)
            .footprint()
            .contains(cell)
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

/// The middle of the cell `target` stands on ([`entity_def::standing_rect`])
/// nearest to a shot released from `origin`: where the shot is aimed, and
/// where a following one lands. Ties go to the first cell in row-major order.
fn aim_cell(world: &World, origin: FixedUVec2, target: Entity) -> FixedUVec2 {
    let projection = world.resource::<Map>().projection();
    let from = CellPos::from(origin);
    let cell = entity_def::standing_rect(world, target)
        .cells()
        .min_by_key(|&cell| projection.metric(from, cell))
        .expect("a footprint has at least one cell");
    cell.middle()
}

/// The entity's simulation id, if it carries one.
fn target_id(world: &World, entity: Entity) -> Option<SimulationId> {
    world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .map(EntityInfoComponent::id)
}
