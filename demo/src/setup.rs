//! One-time scene setup: spawn a base per occupied slot, seed resources, start
//! the session. Runs on entering the game (`OnEnter(GameState::InGame)`) as an
//! exclusive system because spawning needs `&mut World`.

use bevy::prelude::*;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    components::resource::ResourceSourceComponent, resources::PlayerResources,
    session::GameSession, session::player_slot::PlayerId, spawn,
};

use crate::map::{GOLD_MINES, START_POINTS, TREES};

fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// Spawns the starting scene from the session's slots (built by the lobby) and
/// starts the simulation.
///
/// Each occupied slot (human or AI) gets a base playing its chosen race; closed
/// slots are skipped. The slots are byte-identical on every peer, so the scene is
/// too.
pub fn spawn_demo_scene(world: &mut World) {
    // Occupied slots with their chosen race, gathered before mutating the world.
    let occupied: Vec<(PlayerId, String)> = {
        let session = world.resource::<GameSession>();
        session
            .slots()
            .iter()
            .filter(|slot| slot.player_type().is_some())
            .filter_map(|slot| slot.race().map(|race| (slot.id(), race.to_string())))
            .collect()
    };

    for (player, race) in &occupied {
        if (*player as usize) < START_POINTS.len() {
            spawn_base(world, *player, race, START_POINTS[*player as usize]);
        }
    }

    for &(x, y) in &GOLD_MINES {
        if let Some((entity, _)) = spawn::spawn_entity(world, "gold_mine", cell(x, y), None)
            && let Some(mut source) = world
                .entity_mut(entity)
                .get_mut::<ResourceSourceComponent>()
        {
            source.amount = 5000;
        }
    }

    for &(x, y) in TREES {
        if let Some((entity, _)) = spawn::spawn_entity(world, "tree", cell(x, y), None)
            && let Some(mut source) = world
                .entity_mut(entity)
                .get_mut::<ResourceSourceComponent>()
        {
            source.amount = 400;
        }
    }

    {
        let mut resources = world.resource_mut::<PlayerResources>();
        for (player, _) in &occupied {
            resources.add(*player, "gold", 500);
            resources.add(*player, "wood", 200);
        }
    }

    world.resource_mut::<GameSession>().start();
}

fn spawn_base(world: &mut World, player: PlayerId, race: &str, (x, y): (u32, u32)) {
    let (hall, worker) = match race {
        "human" => ("town_hall", "peasant"),
        _ => ("great_hall", "peon"),
    };
    let mut place = |type_name: &str, x: u32, y: u32| {
        spawn::spawn_entity(world, type_name, cell(x, y), Some(player))
            .unwrap_or_else(|| panic!("base cell ({x},{y}) for '{type_name}' must be free"));
    };
    place(hall, x, y);
    place(worker, x + 3, y);
    place(worker, x + 3, y + 1);
}
