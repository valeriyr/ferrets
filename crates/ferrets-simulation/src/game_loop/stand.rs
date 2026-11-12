//! What an entity does once, the first tick it stands finished.

use bevy_ecs::world::World;

use crate::{
    components::{
        hidden::HiddenComponent,
        rise::RisingComponent,
        stand::{StandComponent, Standing},
    },
    entity_def,
    entity_index::EntityIndex,
    field_cover,
};
use ferrets_content::stand::StandingAct;

/// Performs the standing acts of every entity that has come to stand finished
/// since it last acted, and marks them done.
///
/// Standing means alive, on the map, and risen. Entities are visited in
/// ascending simulation-id order, so acts on a shared field land in the same
/// sequence on every peer.
pub fn perform_standing_acts(world: &mut World) {
    let standing: Vec<_> = world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();
    for entity in standing {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<HiddenComponent>() || entity_ref.contains::<RisingComponent>() {
            continue;
        }
        match entity_ref.get::<StandComponent>() {
            Some(StandComponent(Standing::Pending)) => {}
            Some(StandComponent(Standing::Done)) | None => continue,
        }
        let Some(player) = entity_def::owner(world, entity) else {
            continue;
        };
        let footprint = entity_def::location(world, entity).footprint();
        let acts = entity_def::of(world, entity).on_stand.clone();
        for act in acts {
            match act {
                StandingAct::Field {
                    field,
                    area,
                    action,
                } => {
                    let reach = entity_def::area_reach(world, entity, area);
                    field_cover::apply_action_around(
                        world, player, field, footprint, reach, action,
                    );
                }
            }
        }
        world
            .entity_mut(entity)
            .insert(StandComponent(Standing::Done));
    }
}
