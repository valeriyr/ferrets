//! The requirement predicate.
//!
//! Nothing here is stored. Every check re-derives from what stands on the map,
//! what the player has researched and what the actor is at this tick, so a
//! requirement lost when its provider dies and regained when one is rebuilt
//! needs no bookkeeping — the next check sees the current truth.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    annex,
    components::{
        build::UnderConstructionComponent, energy::EnergyComponent,
        entity_info::EntityInfoComponent, entity_stats::StatsComponent, health::HealthComponent,
        order_queue::IdlenessComponent, tags::TagsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    player_research::PlayerResearch,
    session::{GameSession, player_id::PlayerId},
};
use ferrets_content::{
    entity_stats::EntityStatId,
    requirement::{Bound, Requirement},
};

/// Whether `player`, acting through `actor`, currently meets every entry in
/// `requires`, each read as its [`Requirement`] arm says. `actor` is `None` for
/// an act with no acting entity behind it, which no actor-scoped requirement
/// can hold for. An empty list always holds.
pub fn met(
    world: &World,
    player: PlayerId,
    actor: Option<Entity>,
    requires: &[Requirement],
) -> bool {
    requires
        .iter()
        .all(|entry| holds(world, Some(player), actor, entry))
}

/// Whether `actor`, owned by `owner`, currently meets `requirement`, read as
/// in [`met`]. A player-scoped entry holds for no actor that has no owner.
pub fn met_by(
    world: &World,
    owner: Option<PlayerId>,
    actor: Entity,
    requirement: &Requirement,
) -> bool {
    holds(world, owner, Some(actor), requirement)
}

/// Whether one requirement is met — see [`met`].
fn holds(
    world: &World,
    player: Option<PlayerId>,
    actor: Option<Entity>,
    entry: &Requirement,
) -> bool {
    match entry {
        Requirement::All(items) => items.iter().all(|item| holds(world, player, actor, item)),
        Requirement::Any(items) => items.iter().any(|item| holds(world, player, actor, item)),
        Requirement::Research(research) => player.is_some_and(|player| {
            world
                .resource::<PlayerResearch>()
                .is_completed(player, *research)
        }),
        Requirement::Annexed(type_name) => {
            actor.is_some_and(|actor| annex::has_standing_annex(world, actor, type_name))
        }
        Requirement::EntityType(name) => {
            player.is_some_and(|player| standing(world, player, |type_name, _| type_name == name))
        }
        Requirement::Tag(tag) => player.is_some_and(|player| {
            standing(world, player, |_, tags| {
                tags.is_some_and(|tags| tags.contains(tag))
            })
        }),
        Requirement::Health(bound) => actor.is_some_and(|actor| {
            let current = world
                .entity(actor)
                .get::<HealthComponent>()
                .map(HealthComponent::current);
            pool_on(world, actor, current, EntityStatId::MAX_HEALTH, *bound)
        }),
        Requirement::Energy(bound) => actor.is_some_and(|actor| {
            let current = world
                .entity(actor)
                .get::<EnergyComponent>()
                .map(EnergyComponent::current);
            pool_on(world, actor, current, EntityStatId::MAX_ENERGY, *bound)
        }),
        Requirement::Stat { stat, bound } => actor.is_some_and(|actor| {
            let stats = world.entity(actor).get::<StatsComponent>();
            match (
                stats.and_then(|stats| stats.effective(*stat)),
                stats.and_then(|stats| stats.base(*stat)),
            ) {
                (Some(effective), Some(base)) => bound.admits(effective, base),
                (None, _) | (_, None) => false,
            }
        }),
        Requirement::Idle => actor.is_some_and(|actor| entity_def::idle(world, actor)),
        Requirement::IdleFor(ticks) => actor.is_some_and(|actor| {
            entity_def::idle(world, actor)
                && match world.entity(actor).get::<IdlenessComponent>() {
                    Some(IdlenessComponent::IdleSince(since)) => {
                        world
                            .resource::<GameSession>()
                            .tick()
                            .saturating_sub(*since)
                            >= *ticks
                    }
                    Some(IdlenessComponent::Busy) | None => false,
                }
        }),
        Requirement::UnhurtFor(ticks) => actor.is_some_and(|actor| {
            match world
                .entity(actor)
                .get::<HealthComponent>()
                .and_then(HealthComponent::last_hit)
            {
                None => true,
                Some(hit) => {
                    world
                        .resource::<GameSession>()
                        .tick()
                        .saturating_sub(hit.tick)
                        >= *ticks
                }
            }
        }),
    }
}

/// Whether `player` has a standing entity — alive and not under construction —
/// that `matches` its type name and tags. A site still going up unlocks
/// nothing until it stands.
fn standing(
    world: &World,
    player: PlayerId,
    matches: impl Fn(&str, Option<&TagsComponent>) -> bool,
) -> bool {
    world
        .resource::<EntityIndex>()
        .alive_iter()
        .any(|(_, &entity)| {
            let entity_ref = world.entity(entity);
            if entity_def::owner(world, entity) != Some(player)
                || entity_ref.contains::<UnderConstructionComponent>()
            {
                return false;
            }
            let type_name = entity_ref
                .get::<EntityInfoComponent>()
                .expect("simulation entity must have EntityInfoComponent")
                .type_name();
            matches(type_name, entity_ref.get::<TagsComponent>())
        })
}

/// Whether a pool reading `current` lies on the bound's side, a share being of
/// the actor's effective `ceiling` stat. An actor with no such pool lies on
/// neither side.
fn pool_on(
    world: &World,
    actor: Entity,
    current: Option<FixedU64>,
    ceiling: EntityStatId,
    bound: Bound,
) -> bool {
    let (Some(current), Some(max)) = (current, entity_def::effective_stat(world, actor, ceiling))
    else {
        return false;
    };
    bound.admits(current, max)
}
