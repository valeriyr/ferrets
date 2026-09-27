//! Refit of the buffs held on a requirement: every one borne `While`, whoever
//! applied it, and the ones a type bears of itself.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    buffs_store::{Held, Term},
    components::{build::UnderConstructionComponent, entity_buffs::BuffsComponent},
    entity_def,
    entity_index::EntityIndex,
    game_loop::stats,
    requirements,
    session::player_id::PlayerId,
};
use ferrets_content::{
    entity_buffs::{EntityBuffId, Lasting},
    entity_type_def::EntityTypeId,
    registry::ContentRegistry,
};

/// Refits, on every alive entity, the buffs held on a requirement — see
/// [`refit_entity`].
pub fn refit_held_buffs(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        refit_entity(world, entity);
    }
}

/// Drops the passives the type `from` fitted on `entity` that the type it has
/// just changed into does not name. A buff applied by anything else keeps
/// holding on its own requirement.
pub fn shed_form(world: &mut World, entity: Entity, from: EntityTypeId) {
    let registry = world.resource::<ContentRegistry>();
    let now = &entity_def::of(world, entity).passives;
    let buffs = world.entity(entity).get::<BuffsComponent>();
    let dropped: Vec<EntityBuffId> = registry
        .def(from)
        .passives
        .iter()
        .copied()
        .filter(|id| !now.contains(id))
        .filter(|&id| match buffs.and_then(|buffs| buffs.term(id)) {
            Some(Term::While(Held::Passive)) => true,
            Some(Term::While(Held::Applied))
            | Some(Term::Forever)
            | Some(Term::For { .. })
            | Some(Term::Upkeep { .. })
            | None => false,
        })
        .collect();
    for id in dropped {
        stats::remove_entity_buff(world, entity, id);
    }
}

/// Ends each buff `entity` bears that holds `While` a requirement it no longer
/// meets, then applies each passive of its type it does not bear and whose
/// requirement it meets. An entity under construction is fitted no passive.
/// Player-scoped leaves are asked of the bearer's owner, and hold for none when
/// it has none.
pub fn refit_entity(world: &mut World, entity: Entity) {
    let owner = entity_def::owner(world, entity);
    for id in lapsed(world, owner, entity) {
        stats::remove_entity_buff(world, entity, id);
    }
    if world
        .entity(entity)
        .contains::<UnderConstructionComponent>()
    {
        return;
    }
    for id in due(world, owner, entity) {
        stats::fit_passive(world, entity, id);
    }
}

/// The buffs `entity` bears that hold `While` a requirement it no longer
/// meets, in application order.
fn lapsed(world: &World, owner: Option<PlayerId>, entity: Entity) -> Vec<EntityBuffId> {
    let Some(buffs) = world.entity(entity).get::<BuffsComponent>() else {
        return Vec::new();
    };
    let registry = world.resource::<ContentRegistry>();
    buffs
        .active()
        .map(|(id, _)| id)
        .filter(|&id| match &registry.entity_buff_def(id).lasting {
            Lasting::While(requirement) => !requirements::met_by(world, owner, entity, requirement),
            Lasting::Forever | Lasting::For(_) | Lasting::Upkeep { .. } => false,
        })
        .collect()
}

/// The passives of `entity`'s type it does not bear and whose requirement it
/// meets, in the order the type names them.
fn due(world: &World, owner: Option<PlayerId>, entity: Entity) -> Vec<EntityBuffId> {
    let registry = world.resource::<ContentRegistry>();
    entity_def::of(world, entity)
        .passives
        .iter()
        .copied()
        .filter(|&id| !entity_def::bears(world, entity, id))
        .filter(|&id| match &registry.entity_buff_def(id).lasting {
            Lasting::While(requirement) => requirements::met_by(world, owner, entity, requirement),
            Lasting::Forever | Lasting::For(_) | Lasting::Upkeep { .. } => {
                unreachable!("the registry admits only passives that hold on a requirement")
            }
        })
        .collect()
}
