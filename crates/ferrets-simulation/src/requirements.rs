//! The production-requirement predicate.
//!
//! Nothing here is stored. Every check re-derives from what stands on the map
//! and what the player has researched, so a requirement lost when its provider
//! dies and regained when one is rebuilt needs no bookkeeping — the next check
//! sees the current truth.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    annex,
    components::{
        build::UnderConstructionComponent, entity_info::EntityInfoComponent, tags::TagsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    player_research::PlayerResearch,
    session::player_id::PlayerId,
};
use ferrets_content::requirement::Requirement;

/// Whether `player`, acting through `actor`, currently meets every entry in
/// `requires`. `actor` is `None` for an act with no acting entity behind it,
/// which no actor-scoped entry can hold for.
///
/// A research entry holds when the player has completed it. An entity or tag
/// entry holds when the player has a standing entity of that type, or carrying
/// that tag — standing meaning alive and not under construction; a dying
/// entity no longer counts. An annex entry holds when `actor` has a standing
/// annex of that type docked. An empty list always holds.
pub fn met(
    world: &World,
    player: PlayerId,
    actor: Option<Entity>,
    requires: &[Requirement],
) -> bool {
    if requires.is_empty() {
        return true;
    }

    // What can be settled without looking at what stands on the map, first;
    // the names that remain need the pass over the player's entities.
    let mut unmet: Vec<&Requirement> = Vec::new();
    for entry in requires {
        match entry {
            Requirement::Research(research) => {
                if !world
                    .resource::<PlayerResearch>()
                    .is_completed(player, *research)
                {
                    return false;
                }
            }
            Requirement::Annexed(type_name) => {
                let docked =
                    actor.is_some_and(|actor| annex::has_standing_annex(world, actor, type_name));
                if !docked {
                    return false;
                }
            }
            Requirement::EntityType(_) | Requirement::Tag(_) => unmet.push(entry),
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
        unmet.retain(|entry| match entry {
            Requirement::EntityType(name) => name != type_name,
            Requirement::Tag(tag) => !tags.is_some_and(|component| component.contains(tag)),
            Requirement::Research(_) | Requirement::Annexed(_) => {
                unreachable!("settled before the pass over what stands")
            }
        });
        if unmet.is_empty() {
            return true;
        }
    }

    false
}
