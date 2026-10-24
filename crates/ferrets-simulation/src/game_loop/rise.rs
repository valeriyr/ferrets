//! The rise of entities coming into the world, from its ticks of work to
//! its end.

use bevy_ecs::{entity::Entity, query::Without, world::World};
use ferrets_content::{
    field::FieldPlacement,
    registry::ContentRegistry,
    skills::{EntityCastEffect, SkillCaster},
};

use super::warp;
use crate::{
    components::{
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        rise::{self, RiseAttendance, RiseOrigin, RisingComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent},
    fields, rally,
    simulation_id::SimulationId,
    spawn,
};

/// Advances every unattended rising entity by one tick, marking risen the
/// ones that reach their rise time. One a crew works is left to its crew,
/// and a halted one waits.
///
/// Rising entities are visited in ascending simulation-id order.
pub fn advance_unattended(world: &mut World) {
    let mut unattended: Vec<(SimulationId, Entity, SimulationId)> = world
        .query_filtered::<(Entity, &EntityInfoComponent, &RisingComponent), Without<DyingComponent>>()
        .iter(world)
        .filter_map(|(entity, info, rising)| match rising.attendance {
            RiseAttendance::Crew { .. } | RiseAttendance::Halted => None,
            RiseAttendance::Unattended { founder } => Some((info.id(), entity, founder)),
        })
        .collect();
    unattended.sort_unstable_by_key(|&(id, _, _)| id);
    for (_, entity, founder) in unattended {
        let rise_time = entity_def::of(world, entity)
            .rise_time()
            .expect("an unattended rising entity is of a type that rises");

        match put_in_work(world, entity, rise_time, founder) {
            Worked::Going | Worked::Risen => {}
        }
    }
}

/// Lines up each pool of every rising entity that follows its work under the
/// maxima just folded — see [`rise::rise`] — and settles the pools still
/// following work on each entity risen since the last fold — see
/// [`rise::settle_risen`].
pub fn follow_work(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !entity_def::follows_work(world, entity) {
            continue;
        }
        if entity_def::rising(world, entity) {
            rise::rise(world, entity);
        } else {
            rise::settle_risen(world, entity);
        }
    }
}

/// Ends each rise bound to field rules whose footprint has left them, judged
/// for its owner and counting any field it projects itself: a unit warping
/// in, held to its warper type's rules, is called off — see
/// [`warp::call_off`]; a summon rising, held to its skill's, is lost.
///
/// Rising entities are visited in ascending simulation-id order.
pub fn lose_outside_fields(world: &mut World) {
    let lost: Vec<(Entity, RiseOrigin)> = world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter_map(|(_, entity)| {
            let origin = entity_def::rise_origin(world, entity)?;
            let registry = world.resource::<ContentRegistry>();
            let within = match origin {
                RiseOrigin::Founded { .. } => return None,
                RiseOrigin::Warped { warper_type, .. } => registry
                    .def(warper_type)
                    .warper
                    .as_ref()
                    .expect("a unit is warped in by a type that warps")
                    .within(),
                RiseOrigin::Summoned { skill } => {
                    match registry.skill_def(skill).map(|def| &def.caster) {
                        Some(SkillCaster::Entity {
                            effect: EntityCastEffect::Summon { within, .. },
                            ..
                        }) => within,
                        Some(SkillCaster::Entity { .. } | SkillCaster::Player { .. }) | None => {
                            unreachable!("a rising summon is called up by a registered summon")
                        }
                    }
                }
            };
            fields::debug_assert_spreads_on(
                registry,
                entity_def::of(world, entity),
                within.iter().map(FieldPlacement::field),
            );
            let footprint = entity_def::footprint_rect(world, entity);
            (!fields::rules_allow(world, entity_def::owner(world, entity), within, footprint))
                .then_some((entity, origin))
        })
        .collect();
    for (entity, origin) in lost {
        match origin {
            RiseOrigin::Warped { .. } => warp::call_off(world, entity),
            RiseOrigin::Summoned { .. } => {
                spawn::despawn_entity(world, entity, DeathCause::Canceled);
            }
            RiseOrigin::Founded { .. } => unreachable!("a founded rise is bound to no field"),
        }
    }
}

/// What a tick of work did to a rising entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Worked {
    /// The entity goes on rising.
    Going,
    /// The work reached the rise time and the entity has risen.
    Risen,
}

/// Puts one tick of work into the rising `entity`: each pool following its
/// work moves along its line, and once the work reaches `rise_time` the
/// entity has risen, named for `by`.
pub(super) fn put_in_work(
    world: &mut World,
    entity: Entity,
    rise_time: u32,
    by: SimulationId,
) -> Worked {
    let progress = {
        let mut rising = world
            .get_mut::<RisingComponent>(entity)
            .expect("work is put into an entity still rising");
        rising.progress += 1;
        rising.progress
    };
    if entity_def::follows_work(world, entity) {
        rise::rise(world, entity);
    }
    if progress >= rise_time {
        finish_rise(world, entity, by);
        Worked::Risen
    } else {
        Worked::Going
    }
}

/// Marks `entity` as risen — see [`rise::mark_as_risen`] — and announces it,
/// naming `by`: whoever worked the last tick, or the founder of an entity
/// that rose unattended. A risen mover is sent on to its rally — see
/// [`rally::send_on_risen`].
fn finish_rise(world: &mut World, entity: Entity, by: SimulationId) {
    let rising = world
        .get::<RisingComponent>(entity)
        .expect("a rise is finished on an entity still rising");
    let origin = rising.origin;
    // A crewed rise has no one founder to fall back on.
    let founder = match rising.attendance {
        RiseAttendance::Unattended { founder } => Some(founder),
        RiseAttendance::Crew { .. } => None,
        RiseAttendance::Halted => unreachable!("a halted rise takes no work"),
    };
    rise::mark_as_risen(world, entity);
    let announced = SimulationEvent::EntityRisen {
        entity: entity_def::simulation_id(world, entity),
        by,
        origin,
    };
    world.resource_mut::<EventRecord>().emit(announced);
    if entity_def::of(world, entity).can_move() {
        rally::send_on_risen(world, entity, founder);
    }
}
