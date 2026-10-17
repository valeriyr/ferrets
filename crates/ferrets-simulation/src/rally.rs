//! Rally points: where a holder sends the units it releases, and the marker
//! a released unit carries until the dispatch pass sends it.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    components::rally::{RallyDueComponent, RallyPointComponent, RallyTarget},
    entity_def,
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

/// Marks a freshly released `unit` due a dispatch to `target`.
pub(crate) fn mark_rally_due_to(world: &mut World, unit: Entity, target: RallyTarget) {
    world.entity_mut(unit).insert(RallyDueComponent { target });
}
