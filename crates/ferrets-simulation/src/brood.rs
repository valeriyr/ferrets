//! The brood a breeder counts: who is on it, what it is owed, the terms it
//! breeds on, and the tie a broodling's own change of form ends.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    brood::{BreederDef, BroodlingDef},
    entity_type_def::EntityTypeId,
    registry::ContentRegistry,
    work::Attachment,
};

use crate::{
    berths,
    components::{
        brood::{BredComponent, BroodComponent},
        rally::RallyTarget,
    },
    entity_def,
    entity_index::EntityIndex,
    rally,
    simulation_id::SimulationId,
};

//
// ─── The count ──────────────────────────────────────────────────────────────
//

/// Ties `broodling` to `breeder`: bred by it, and on its brood.
///
/// Panics if the breeder keeps no brood: a broodling is tied only to a
/// breeder.
pub(crate) fn tie(world: &mut World, breeder: Entity, broodling: Entity) {
    let by = entity_def::simulation_id(world, breeder);
    let id = entity_def::simulation_id(world, broodling);
    world.entity_mut(broodling).insert(BredComponent { by });
    add(world, breeder, id);
}

/// Cuts `broodling`'s tie to `breeder`: no longer bred by it, off its brood.
pub(crate) fn untie(world: &mut World, breeder: Entity, broodling: Entity) {
    let id = entity_def::simulation_id(world, broodling);
    world.entity_mut(broodling).remove::<BredComponent>();
    remove(world, breeder, id);
}

/// Adds `broodling` to `breeder`'s brood, once.
///
/// Panics if the breeder keeps no brood: a broodling is added only to a
/// breeder's.
pub(crate) fn add(world: &mut World, breeder: Entity, broodling: SimulationId) {
    let mut breeder = world.entity_mut(breeder);
    let mut brood = breeder
        .get_mut::<BroodComponent>()
        .expect("a broodling is added only to a breeder's brood");
    if !brood.broodlings.contains(&broodling) {
        brood.broodlings.push(broodling);
    }
}

/// Takes `broodling` out of `breeder`'s brood. Nothing when the brood does not
/// hold it, or the form the breeder has just taken breeds nothing and its
/// brood went with the landing, ahead of its broodlings.
pub(crate) fn remove(world: &mut World, breeder: Entity, broodling: SimulationId) {
    if let Some(mut brood) = world.entity_mut(breeder).get_mut::<BroodComponent>() {
        brood.broodlings.retain(|held| *held != broodling);
    }
}

/// Owes `breeder` the births that bring its brood up to what the form it has
/// just taken opens with. Nothing for a form that breeds nothing, or a brood
/// already that large.
pub(crate) fn open(world: &mut World, breeder: Entity) {
    let Some(initial) = entity_def::of(world, breeder)
        .breeder
        .as_ref()
        .map(BreederDef::initial)
    else {
        return;
    };
    let mut breeder = world.entity_mut(breeder);
    let mut brood = breeder
        .get_mut::<BroodComponent>()
        .expect("a breeding form is fitted a brood");
    brood.owed = u32::try_from(initial.saturating_sub(brood.broodlings.len()))
        .expect("an initial brood fits in u32");
}

/// The brood `entity` keeps — its timer and its broodlings — or `None` for an
/// entity that breeds nothing.
pub fn of(world: &World, entity: Entity) -> Option<&BroodComponent> {
    world.entity(entity).get::<BroodComponent>()
}

/// The broodlings in `entity`'s brood, in the order they joined it; none for an
/// entity that breeds nothing.
pub fn broodlings(world: &World, entity: Entity) -> &[SimulationId] {
    of(world, entity).map_or(&[], |brood| brood.broodlings.as_slice())
}

/// The terms `entity` breeds on: the ones the form it wears declares, or, while
/// it wears an interim form on its way to another, the ones the form it
/// changes from declared. `None` for an entity that breeds nothing.
pub fn terms(world: &World, entity: Entity) -> Option<&BreederDef> {
    terms_on(world, entity, entity_def::morph_origin(world, entity))
}

/// The terms `entity` breeds on with `origin` as the form its change was
/// declared on: the ones the form it wears declares, else `origin`'s. `None`
/// when neither breeds.
pub(crate) fn terms_on(world: &World, entity: Entity, origin: EntityTypeId) -> Option<&BreederDef> {
    if let Some(breeder) = &entity_def::of(world, entity).breeder {
        return Some(breeder);
    }
    world
        .resource::<ContentRegistry>()
        .def(origin)
        .breeder
        .as_ref()
}

/// How `entity`'s type sits in its breeder's berths, or `None` when nothing
/// breeds it.
pub fn broodling_attachment(world: &World, entity: Entity) -> Option<&Attachment> {
    entity_def::of(world, entity)
        .broodling
        .as_ref()
        .map(BroodlingDef::attachment)
}

//
// ─── A bred entity changes form ─────────────────────────────────────────────
//

/// Takes a bred entity off its breeder's count as it steps out of its berth to
/// change, so the next birth is on its way; the tie stays for the change's
/// end. Nothing for an entity nobody bred.
pub(crate) fn uncount(world: &mut World, entity: Entity) {
    let Some(bred) = world.entity(entity).get::<BredComponent>().cloned() else {
        return;
    };
    let id = entity_def::simulation_id(world, entity);
    if let Some(breeder) = world.resource::<EntityIndex>().alive(bred.by) {
        remove(world, breeder, id);
    }
}

/// Hatches `entity`, a landed change having made a unit of it: a bred
/// entity's tie to its breeder ends, and the unit is sent to `own` — the
/// rally point the changing form carried — or, failing that, to its breeder's.
/// An entity nobody bred is sent to `own` alone.
pub(crate) fn hatch(world: &mut World, entity: Entity, own: Option<RallyTarget>) {
    let bred = world.entity_mut(entity).take::<BredComponent>();
    let target = own.or_else(|| {
        let breeder = world.resource::<EntityIndex>().alive(bred?.by)?;
        rally::target_of(world, breeder)
    });
    if let Some(target) = target {
        rally::owe_target(world, entity, target);
    }
}

//
// ─── Seats ──────────────────────────────────────────────────────────────────
//

/// Whether `breeder`, in the form it wears, has a seat for `entity` in the
/// group `attachment` names.
pub(crate) fn holds(
    world: &World,
    breeder: Entity,
    entity: Entity,
    attachment: &Attachment,
) -> bool {
    let standing = entity_def::standing_rect(world, entity);
    berths::offers(world, breeder, attachment.berths())
        && berths::nearest_free_seat(world, breeder, attachment.berths(), standing).is_some()
}
