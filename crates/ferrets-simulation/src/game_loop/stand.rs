//! What an entity does once, the first tick it stands finished.

use bevy_ecs::world::World;

use crate::{
    components::{
        build::UnderConstructionComponent,
        hidden::HiddenComponent,
        stand::{StandComponent, Standing},
    },
    entity_def,
    entity_index::EntityIndex,
    game_loop::fields,
};
use ferrets_content::stand::StandingAct;

/// Performs the standing acts of every entity that has come to stand finished
/// since it last acted, and marks them done.
///
/// Standing means alive, on the map, and not under construction. Entities are
/// visited in ascending simulation-id order, so acts on a shared field land in
/// the same sequence on every peer.
pub fn perform_standing_acts(world: &mut World) {
    let standing: Vec<_> = world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();
    for entity in standing {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<HiddenComponent>()
            || entity_ref.contains::<UnderConstructionComponent>()
        {
            continue;
        }
        match entity_ref.get::<StandComponent>() {
            Some(StandComponent(Standing::Pending)) => {}
            Some(StandComponent(Standing::Done)) | None => continue,
        }
        let Some(player) = entity_def::owner(world, entity) else {
            continue;
        };
        let footprint = entity_def::occupied_rect(world, entity);
        let acts = entity_def::of(world, entity).on_stand.clone();
        for act in acts {
            match act {
                StandingAct::Field {
                    field,
                    radius,
                    action,
                } => fields::apply_action_around(world, player, field, footprint, radius, action),
            }
        }
        world
            .entity_mut(entity)
            .insert(StandComponent(Standing::Done));
    }
}
