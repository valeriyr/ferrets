//! Folding the tick's announcements into the per-player tallies.
//!
//! The one place that knows what counts as a statistic.

use std::collections::BTreeMap;

use bevy_ecs::{change_detection::Mut, world::World};

use crate::{
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause},
    session::{GameSession, player_slot::PlayerId},
    simulation_id::SimulationId,
    statistics::Statistics,
};

/// Folds everything the tick announced into the tallies.
///
/// What an entity is and whose it is are read off the entity an announcement
/// names, which may already be dying — a tick can produce something and kill it
/// before it closes.
pub fn collect(world: &mut World) {
    world.resource_scope(|world, mut statistics: Mut<Statistics>| {
        // Every death the tick announced, by subject. A passenger's death names
        // only the carrier it was inside; the carrier's own death carries the
        // enemy behind both. Collected whole before any folding, so attribution
        // does not depend on the order the two were announced in.
        let mut deaths: BTreeMap<SimulationId, DeathCause> = BTreeMap::new();
        for event in world.resource::<EventRecord>().events() {
            if let SimulationEvent::EntityDied { entity, cause, .. } = event {
                deaths.insert(*entity, *cause);
            }
        }
        for event in world.resource::<EventRecord>().events() {
            fold(world, event, &deaths, &mut statistics);
        }
    });
}

/// Folds one announcement.
fn fold(
    world: &World,
    event: &SimulationEvent,
    deaths: &BTreeMap<SimulationId, DeathCause>,
    statistics: &mut Statistics,
) {
    match event {
        SimulationEvent::EntitySpawned { entity, cause } => {
            // Only what a player finished counts as production: a map placement
            // or a sandbox conjuring was not their doing, remains never are, and
            // a founded site is counted by its completion instead.
            let produced = match cause {
                SpawnCause::Trained { .. } => true,
                SpawnCause::Placed
                | SpawnCause::Founded { .. }
                | SpawnCause::Sandbox
                | SpawnCause::Remains { .. } => false,
            };
            if !produced {
                return;
            }
            let Some(spawned) = world.resource::<EntityIndex>().any(*entity) else {
                return;
            };
            if let Some(player) = entity_def::owner(world, spawned) {
                statistics.record_produced(player, entity_def::type_id(world, spawned));
            }
        }
        SimulationEvent::ConstructionCompleted { building, .. } => {
            let Some(finished) = world.resource::<EntityIndex>().any(*building) else {
                return;
            };
            if let Some(player) = entity_def::owner(world, finished) {
                statistics.record_produced(player, entity_def::type_id(world, finished));
            }
        }
        SimulationEvent::EntityDied {
            entity_type,
            owner,
            cause,
            ..
        } => {
            // A loss is something fire took: an owner cancelling its own
            // construction, or a resource node running dry, is neither a loss
            // nor anyone's kill. A passenger's death traces to whoever brought
            // its carrier down.
            let Some(by_owner) = fire_behind(*cause, deaths) else {
                return;
            };
            if let Some(player) = owner {
                statistics.record_lost(*player, *entity_type);
            }
            if let Some(killer) = by_owner {
                // Downing your own or an ally's earns no kill; the victim
                // already counted it as a loss.
                if !same_side(world, killer, *owner) {
                    statistics.record_killed(killer, *entity_type);
                }
            }
        }
        SimulationEvent::DamageLanded {
            target_owner,
            attacker_owner,
            amount,
            ..
        } => {
            if let Some(player) = target_owner {
                statistics.record_damage_taken(*player, *amount);
            }
            if let Some(player) = attacker_owner {
                // Damage to your own or an ally's is taken but never dealt.
                if !same_side(world, *player, *target_owner) {
                    statistics.record_damage_dealt(*player, *amount);
                }
            }
        }
        SimulationEvent::ResourcesGathered {
            player,
            kind,
            amount,
            ..
        } => statistics.record_gathered(*player, kind, *amount),
        // The cause is carried for a reader that wants spending broken down by
        // reason; these totals are per resource kind, so it is not folded in
        // here.
        SimulationEvent::ResourcesSpent { player, cost, .. } => {
            for (kind, amount) in cost {
                statistics.record_spent(*player, kind, *amount);
            }
        }
        SimulationEvent::ResourcesRefunded { player, cost, .. } => {
            for (kind, amount) in cost {
                statistics.record_refunded(*player, kind, *amount);
            }
        }
        SimulationEvent::ResearchCompleted { player, .. } => statistics.record_research(*player),
        SimulationEvent::SkillCast { caster, .. } => {
            let Some(entity) = world.resource::<EntityIndex>().any(*caster) else {
                return;
            };
            if let Some(player) = entity_def::owner(world, entity) {
                statistics.record_skill_cast(player);
            }
        }
        SimulationEvent::PlayerSkillCast { player, .. } => statistics.record_skill_cast(*player),
        // Neither a form change nor going off the map and back is a thing a
        // tally counts: the entity was already counted when it was made, and a
        // form that switches back and forth would count every switch.
        SimulationEvent::EntityMorphed { .. }
        | SimulationEvent::EntityHidden { .. }
        | SimulationEvent::EntityRevealed { .. } => {}
    }
}

/// Whether `player` and `other` stand on one side — the same player, or allies.
/// An unowned `other` is on nobody's side.
fn same_side(world: &World, player: PlayerId, other: Option<PlayerId>) -> bool {
    match other {
        Some(other) => world.resource::<GameSession>().are_allied(player, other),
        None => false,
    }
}

/// The owner behind the killing hit — `Some(None)` for a neutral attacker — or
/// `None` for a death no fire caused.
///
/// A passenger lost with its carrier died to whatever took the carrier down, so
/// the chain of holders is followed to the death that started it. A holder with
/// no announced death of its own ends the chase: nothing traces the loss to a
/// shot.
fn fire_behind(
    mut cause: DeathCause,
    deaths: &BTreeMap<SimulationId, DeathCause>,
) -> Option<Option<PlayerId>> {
    loop {
        match cause {
            DeathCause::Killed { by_owner, .. } => return Some(by_owner),
            DeathCause::PassengerLost { holder } => match deaths.get(&holder) {
                Some(holder_cause) => cause = *holder_cause,
                None => return None,
            },
            DeathCause::Depleted | DeathCause::Cancelled => return None,
        }
    }
}
