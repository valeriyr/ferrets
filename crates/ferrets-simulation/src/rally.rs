//! Rally points: where a holder sends the units it releases, and the marker
//! a released unit carries until the dispatch pass sends it.

use bevy_ecs::{entity::Entity, world::World};

use crate::components::rally::{RallyDueComponent, RallyPointComponent, RallyTarget};

/// Where `holder`'s rally point sends units, or `None` when it carries none or
/// has none set.
pub fn target_of(world: &World, holder: Entity) -> Option<RallyTarget> {
    world
        .entity(holder)
        .get::<RallyPointComponent>()
        .and_then(|rally| rally.0)
}

/// Owes a freshly released `unit` the dispatch of `holder`'s rally point, if
/// one is set.
pub(crate) fn owe(world: &mut World, holder: Entity, unit: Entity) {
    if let Some(target) = target_of(world, holder) {
        owe_target(world, unit, target);
    }
}

/// Owes a freshly released `unit` a dispatch to `target`.
pub(crate) fn owe_target(world: &mut World, unit: Entity, target: RallyTarget) {
    world.entity_mut(unit).insert(RallyDueComponent { target });
}
