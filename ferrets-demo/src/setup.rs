//! One-time scene setup: build the map with its neutral placements, spawn a
//! base per occupied slot, seed resources, start the session. Runs on entering
//! the game (`OnEnter(GameState::InGame)`) as an exclusive system because
//! spawning needs `&mut World`.

use bevy::prelude::*;
use ferrets_bevy_plugin::instantiate_map;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    content::player_stats::PlayerStatId,
    map::Map,
    player_stats::PlayerStats,
    resources::PlayerResources,
    session::{GameSession, player_slot::PlayerId},
    spawn,
};

use crate::{map, settings::Settings};

fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// Spawns the starting scene from the chosen map and the session's slots
/// (built by the lobby), and starts the simulation.
///
/// The map contributes its own placements (the neutral mines and groves); each
/// occupied slot (human or AI) then gets a base playing its chosen race at its
/// start point, and closed slots are skipped. The slots are byte-identical on
/// every peer, so the scene is too.
pub fn spawn_demo_scene(world: &mut World) {
    // Occupied slots with their chosen race, gathered before mutating the world.
    let occupied: Vec<(PlayerId, String)> = {
        let session = world.resource::<GameSession>();
        session
            .occupied_slots()
            .filter_map(|slot| slot.race().map(|race| (slot.id(), race.to_string())))
            .collect()
    };

    // The session names the map; every entry path validated the name, so a
    // miss here is a configuration bug, not user input.
    let name = world.resource::<GameSession>().map().to_string();
    let mut map =
        map::by_name(&name).unwrap_or_else(|| panic!("the session names an unknown map '{name}'"));
    // The menu's settings shape the game; a headless harness without them
    // keeps whatever map it installed itself.
    let (model, projection) = world.get_resource::<Settings>().map_or_else(
        || {
            let map = world.resource::<Map>();
            (map.movement_model(), map.projection())
        },
        |settings| (settings.movement_model, settings.view.projection()),
    );
    map.set_movement_model(model);
    map.set_projection(projection);
    instantiate_map(world, &map);

    for (player, race) in &occupied {
        if let Some(start) = world.resource::<Map>().start_point(*player) {
            spawn_base(world, *player, race, (start.x, start.y));
        }
    }

    {
        let mut resources = world.resource_mut::<PlayerResources>();
        for (player, _) in &occupied {
            resources.add(*player, "gold", 500);
            resources.add(*player, "wood", 200);
        }
    }

    seed_player_stats(world);

    world.resource_mut::<GameSession>().start();
}

/// Seeds the demo's baseline player stats for every occupied slot: a hard
/// supply ceiling well above what a map's farms can provide, so the headroom
/// players actually play against comes from their standing buildings.
///
/// Every game mode's spawner runs this before starting the session, so the
/// ceiling holds however the session was configured.
pub fn seed_player_stats(world: &mut World) {
    let players: Vec<PlayerId> = {
        let session = world.resource::<GameSession>();
        session.occupied_slots().map(|slot| slot.id()).collect()
    };
    let mut stats = world.resource_mut::<PlayerStats>();
    for player in players {
        stats.set_base(player, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(200));
    }
}

fn spawn_base(world: &mut World, player: PlayerId, race: &str, (x, y): (u32, u32)) {
    let (hall, worker) = match race {
        "human" => ("town_hall", "peasant"),
        _ => ("great_hall", "peon"),
    };
    let mut place = |type_name: &str, x: u32, y: u32| {
        if spawn::spawn_entity(world, type_name, cell(x, y), Some(player)).is_none() {
            eprintln!("base cell ({x},{y}) cannot host '{type_name}'; spawn skipped");
        }
    };
    place(hall, x, y);
    place(worker, x + 3, y);
    place(worker, x + 3, y + 1);
}
