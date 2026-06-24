//! One-time scene setup: assign races, spawn both bases, seed resources, start
//! the session. Runs on entering the game (`OnEnter(GameState::InGame)`) as an
//! exclusive system because spawning needs `&mut World`.

use bevy::prelude::*;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    components::resource::ResourceSourceComponent, input::InputFrames, resources::PlayerResources,
    session::GameSession, session::player_slot::PlayerId, spawn,
};

use crate::map::{GOLD_MINES, START_POINTS, TREES};
use crate::states::ChosenRace;

fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// The race the local player did not pick, given to the passive enemy.
fn opposing(race: &str) -> &'static str {
    if race == "human" { "orc" } else { "human" }
}

/// Assigns races, spawns the starting scene, and starts the simulation.
///
/// Player 0 (local) plays the race chosen in the menu; player 1 (the passive
/// enemy) plays the other one. Each base is spawned from its slot's race, so the
/// scene is driven entirely by session data.
pub fn spawn_demo_scene(world: &mut World) {
    let chosen = world.resource::<ChosenRace>().0.clone();
    {
        let mut session = world.resource_mut::<GameSession>();
        session.set_race(0, chosen.clone());
        session.set_race(1, opposing(&chosen));
    }

    for player in 0..START_POINTS.len() as PlayerId {
        let race = world
            .resource::<GameSession>()
            .slot(player)
            .and_then(|slot| slot.race())
            .map(String::from);
        if let Some(race) = race {
            spawn_base(world, player, &race, START_POINTS[player as usize]);
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
        resources.add(0, "gold", 500);
        resources.add(0, "wood", 200);
    }

    world.resource_mut::<GameSession>().start();
}

/// Supplies idle input frames for every non-local player each tick.
///
/// Single-player has no network peer and no AI yet, so without this the lockstep
/// loop would block forever waiting on player 1's frames. (A real game would
/// source these from the network or an AI driver.)
pub fn supply_ai_input(mut frames: ResMut<InputFrames>, session: Res<GameSession>) {
    let tick = session.tick();
    let local = session.local_player();
    for slot in session.slots() {
        if slot.id() != local {
            frames.ensure_idle(slot.id(), tick);
        }
    }
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
