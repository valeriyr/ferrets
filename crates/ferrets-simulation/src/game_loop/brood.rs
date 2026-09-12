//! The breeding pass: the births a breeder owes and the ones its timer brings,
//! and the set-down broodlings it takes in.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    brood::{BreederDef, BroodlingDef, Lingering, OrphanFate},
    registry::ContentRegistry,
};

use crate::{
    brood,
    components::{
        brood::{BredComponent, BroodComponent},
        hidden::HiddenComponent,
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    events::SpawnCause,
    map::Map,
    spawn,
};

//
// ─── Breeding ───────────────────────────────────────────────────────────────
//

/// Advances every breeder one tick, in ascending simulation-id order.
///
/// A breeder breeds on the terms of the form it wears while it operates: it
/// first takes in the set-down broodlings its terms let it, then delivers the
/// births it owes, as far as its limit allows, then counts a tick toward the
/// next birth — holding its progress while as many broodlings live as the
/// limit admits — and bears one when the period runs out.
pub fn advance(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !world.entity(entity).contains::<BroodComponent>() {
            continue;
        }
        let Some(terms) = entity_def::of(world, entity).breeder.clone() else {
            continue;
        };
        match entity_def::operation(world, entity) {
            Operation::Operating => {}
            Operation::UnderConstruction | Operation::Disabled(_) => continue,
        }

        match terms.orphans() {
            OrphanFate::Linger(Lingering::Reseat { distance }) => {
                take_in(world, entity, &terms, distance);
            }
            OrphanFate::Perish | OrphanFate::Linger(Lingering::Stay) => {}
        }

        while brood::of(world, entity)
            .is_some_and(|brood| brood.owed > 0 && brood.broodlings.len() < terms.limit())
        {
            if bear(world, entity, &terms).is_none() {
                break;
            }
            let mut breeder = world.entity_mut(entity);
            let mut brood = breeder
                .get_mut::<BroodComponent>()
                .expect("a breeder keeps a brood");
            brood.owed -= 1;
        }

        if brood::broodlings(world, entity).len() >= terms.limit() {
            continue;
        }
        let period = entity_def::period_ticks(world, entity, terms.period());
        let progress = {
            let mut breeder = world.entity_mut(entity);
            let mut brood = breeder
                .get_mut::<BroodComponent>()
                .expect("a breeder keeps a brood");
            brood.progress = brood.progress.saturating_add(1);
            brood.progress
        };
        if progress >= period {
            let progress = match bear(world, entity, &terms) {
                Some(_) => 0,
                None => period,
            };
            world
                .entity_mut(entity)
                .get_mut::<BroodComponent>()
                .expect("a breeder keeps a brood")
                .progress = progress;
        }
    }
}

//
// ─── Private steps ──────────────────────────────────────────────────────────
//

/// Bears one broodling for `breeder` on `terms`, seated in the breeder's
/// berths as the bred type's attachment says and tied to it, or `None` when
/// the group has no seat for it.
fn bear(world: &mut World, breeder: Entity, terms: &BreederDef) -> Option<Entity> {
    let by = entity_def::simulation_id(world, breeder);
    let owner = entity_def::owner(world, breeder);
    let attachment = world
        .resource::<ContentRegistry>()
        .entity(terms.breeds())
        .expect("validated content breeds registered types")
        .broodling
        .as_ref()
        .map(BroodlingDef::attachment)
        .cloned()
        .expect("validated content breeds broodlings");
    let (broodling, _) = spawn::spawn_seated(
        world,
        terms.breeds(),
        breeder,
        &attachment,
        owner,
        SpawnCause::Bred { by },
    )?;
    brood::tie(world, breeder, broodling);
    Some(broodling)
}

/// Takes set-down broodlings into `breeder`'s brood on `terms`, in ascending
/// simulation-id order, while its limit and its seats allow: each of its
/// player's entities of the bred type that stands untied, idle and on the
/// grid within `distance` cells of the footprint is seated and tied.
fn take_in(world: &mut World, breeder: Entity, terms: &BreederDef, distance: u32) {
    if brood::broodlings(world, breeder).len() >= terms.limit() {
        return;
    }
    let owner = entity_def::owner(world, breeder);
    let projection = world.resource::<Map>().projection();
    let footprint = entity_def::footprint_rect(world, breeder);
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if brood::broodlings(world, breeder).len() >= terms.limit() {
            return;
        }
        if entity_def::of(world, entity).name != terms.breeds()
            || entity_def::owner(world, entity) != owner
        {
            continue;
        }
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<BredComponent>()
            || entity_ref.contains::<HiddenComponent>()
            || !entity_def::stands_on_grid(world, entity)
            || !entity_def::orders(world, entity).is_empty()
            || !projection.in_range_for_rects(
                footprint,
                entity_def::standing_rect(world, entity),
                distance,
            )
        {
            continue;
        }
        // Its own copy of the attachment: the seating below changes the world.
        let attachment = brood::broodling_attachment(world, entity)
            .cloned()
            .expect("a bred type is a broodling");
        if !brood::holds(world, breeder, entity, &attachment) {
            return;
        }
        spawn::attach_entity(world, entity, breeder, &attachment);
        brood::tie(world, breeder, entity);
    }
}
