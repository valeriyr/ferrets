//! The production-requirement predicate.
//!
//! Nothing here is stored. Every check re-derives from what stands on the map
//! and what the player has researched, so a requirement lost when its provider
//! dies and regained when one is rebuilt needs no bookkeeping — the next check
//! sees the current truth.

use bevy_ecs::world::World;

use crate::{
    components::{
        build::UnderConstructionComponent, entity_info::EntityInfoComponent, tags::TagsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    player_research::PlayerResearch,
    session::player_id::PlayerId,
};
use ferrets_content::registry::ContentRegistry;

/// Whether `player` currently meets every entry in `requires`.
///
/// An entry naming a research holds when the player has completed it. Any
/// other entry holds when the player has a standing entity whose type name
/// equals the entry or whose tags contain it — standing meaning alive and not
/// under construction; a dying entity no longer counts. An empty list always
/// holds.
pub fn met(world: &World, player: PlayerId, requires: &[String]) -> bool {
    if requires.is_empty() {
        return true;
    }

    let registry = world.resource::<ContentRegistry>();
    let research = world.resource::<PlayerResearch>();

    // Settle research entries first; what remains needs the entity pass.
    let mut unmet: Vec<&str> = Vec::new();
    for name in requires {
        match registry.research(name) {
            Some(id) => {
                if !research.is_completed(player, id) {
                    return false;
                }
            }
            None => unmet.push(name),
        }
    }
    if unmet.is_empty() {
        return true;
    }

    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_def::owner(world, entity) != Some(player) {
            continue;
        }
        // A site still going up unlocks nothing until it stands.
        if entity_ref.contains::<UnderConstructionComponent>() {
            continue;
        }

        let type_name = entity_ref
            .get::<EntityInfoComponent>()
            .expect("simulation entity must have EntityInfoComponent")
            .type_name();
        let tags = entity_ref.get::<TagsComponent>();
        unmet.retain(|name| {
            *name != type_name && !tags.is_some_and(|component| component.contains(name))
        });
        if unmet.is_empty() {
            return true;
        }
    }

    false
}
