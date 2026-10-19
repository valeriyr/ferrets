//! Rally points: where a holder sends the units it releases, where a mover
//! still rising goes once it has risen, and the marker such a unit carries
//! until the dispatch pass sends it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{entity_type_def::EntityTypeDef, registry::ContentRegistry};

use crate::{
    components::rally::{RallyDueComponent, RallyPointComponent, RallyTarget},
    entity_def,
    entity_index::EntityIndex,
    simulation_id::SimulationId,
};

/// Where `holder`'s rally point sends units, or `None` when it carries none or
/// has none set.
pub fn target_of(world: &World, holder: Entity) -> Option<RallyTarget> {
    world
        .entity(holder)
        .get::<RallyPointComponent>()
        .and_then(|rally| rally.0)
}

/// Marks a freshly released `unit` due the dispatch of `holder`'s rally
/// point, if one is set and `unit` belongs to whoever owns `holder`: a holder
/// sends on only its own owner's units.
pub(crate) fn mark_rally_due(world: &mut World, holder: Entity, unit: Entity) {
    if !entity_def::same_owner(world, holder, unit) {
        return;
    }
    if let Some(target) = target_of(world, holder) {
        mark_rally_due_to(world, unit, target);
    }
}

/// Marks a freshly released or risen `unit` due a dispatch to `target`.
pub(crate) fn mark_rally_due_to(world: &mut World, unit: Entity, target: RallyTarget) {
    world.entity_mut(unit).insert(RallyDueComponent { target });
}

/// Sends `entity`, a mover that has just risen, on to its own rally, else to
/// the rally of its `founder` — whatever brought it in, if it still stands,
/// shares its owner and has one set — and takes its own rally off: a rally
/// set on a rising mover is where it goes, kept on as an empty rally point
/// only by a type that releases units (see [`releases_units`]).
///
/// Panics when `entity` carries no rally point.
pub(crate) fn send_on_risen(world: &mut World, entity: Entity, founder: Option<SimulationId>) {
    let own = world
        .get_mut::<RallyPointComponent>(entity)
        .expect("a mover rising carries a rally point")
        .0
        .take();
    let keeps = {
        let registry = world.resource::<ContentRegistry>();
        releases_units(registry, entity_def::of(world, entity))
    };
    if !keeps {
        world.entity_mut(entity).remove::<RallyPointComponent>();
    }
    let target = own.or_else(|| {
        let founder = world.resource::<EntityIndex>().alive(founder?)?;
        entity_def::same_owner(world, founder, entity)
            .then(|| target_of(world, founder))
            .flatten()
    });
    if let Some(target) = target {
        mark_rally_due_to(world, entity, target);
    }
}

/// Whether a type of `def` releases units and keeps a rally point for them:
/// it trains, warps in, carries passengers, joins a gateway network, or
/// breeds broodlings that make units by a change of form.
pub(crate) fn releases_units(registry: &ContentRegistry, def: &EntityTypeDef) -> bool {
    def.trainer.is_some()
        || def.warper.is_some()
        || def.can_transport()
        || def.gateway().is_some()
        || def.breeder.as_ref().is_some_and(|breeder| {
            registry
                .entity(breeder.breeds())
                .expect("validated content breeds registered types")
                .produces_by_morph()
        })
}
