//! Garrisoned passengers fighting from inside their holder.
//!
//! A holder that lets its passengers attack turns every armed passenger into an
//! emplacement: each acquires and works its own target with its own weapon, but
//! stands nowhere — every range and fog reading is the holder's, and every shot
//! leaves from the holder. Passengers take no orders while hidden, so this runs
//! outside the order lifecycle, on the holder's initiative alone.

use bevy_ecs::{entity::Entity, world::World};

use super::{acquire, attack};
use crate::{
    components::transport::{GarrisonFireComponent, TransporterComponent},
    content::transport::PassengerConduct,
    entity_def,
    entity_index::EntityIndex,
    session::GameSession,
    simulation_id::SimulationId,
};

/// Advances every garrisoned attacker's fight by one tick.
///
/// Holders are visited in id order and their passengers in id order, so every
/// peer fires the same shots in the same sequence.
pub fn process_garrison_attacks(world: &mut World) {
    let holders: Vec<Entity> = world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();

    for holder in holders {
        let conduct = entity_def::of(world, holder)
            .transporter
            .as_ref()
            .map(|transporter| transporter.conduct());
        match conduct {
            Some(PassengerConduct::Fight) => {}
            Some(PassengerConduct::Shelter) | None => continue,
        }
        let passenger_ids: Vec<SimulationId> = world
            .entity(holder)
            .get::<TransporterComponent>()
            .map(|transporter| transporter.passengers.iter().copied().collect())
            .unwrap_or_default();
        for passenger_id in passenger_ids {
            let Some(passenger) = world.resource::<EntityIndex>().alive(passenger_id) else {
                continue;
            };
            if !entity_def::of(world, passenger).can_attack() {
                continue;
            }
            fire(world, holder, passenger, passenger_id);
        }
    }
}

/// One tick of one garrisoned attacker's fight: hold a valid target or look for
/// one, and advance the swing while it has one.
fn fire(world: &mut World, holder: Entity, passenger: Entity, passenger_id: SimulationId) {
    let weapon = attack::weapon(world, passenger);

    let mut fire = world
        .entity_mut(passenger)
        .take::<GarrisonFireComponent>()
        .unwrap_or_default();

    // The holder is the one standing on the map, so it is the seeker every
    // acquisition reading runs for: range from its footprint, fog by its owner,
    // and its own fresh attacker preferred — a bunker under fire shoots back.
    let valid = fire
        .target
        .is_some_and(|id| acquire::qualifies(world, holder, id, weapon.range));
    if !valid {
        fire.phase = 0;
        let tick = world.resource::<GameSession>().tick();
        fire.target = if acquire::due(passenger_id, tick) {
            acquire::find_target(world, holder, weapon.range)
        } else {
            None
        };
    }

    let Some(target_id) = fire.target else {
        world.entity_mut(passenger).insert(fire);
        return;
    };

    let target = world
        .resource::<EntityIndex>()
        .alive(target_id)
        .expect("a qualifying target is alive");
    let (target_position, _) = entity_def::footprint(world, target);
    let origin = entity_def::position(world, holder);
    attack::swing(
        world,
        passenger,
        origin,
        Some(target),
        target_position,
        &weapon,
        &mut fire.phase,
    );

    world.entity_mut(passenger).insert(fire);
}
