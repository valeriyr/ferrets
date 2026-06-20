//! ferrets demo — simulation driven by a Bevy app.
//!
//! Demonstrates the ferrets-bevy integration:
//!   1. register content types in Startup
//!   2. spawn units by type name and position
//!   3. push attack and move commands via PendingInput
//!   4. read sim state directly via Query in FixedUpdate
use bevy::prelude::*;
use ferrets_bevy::{PendingInput, SimulationPlugin, SimulationSet};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    astar::Projection, nav_grid::NavGrid, nav_pos::NavPos, nav_size::NavSize,
};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::{LocationComponent, Solidity},
    },
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    map::Map,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

/// Navigation layer IDs for this game. Each value is a non-zero power of two.
mod nav_layer {
    use ferrets_pathfinder::nav_grid::LayerId;
    pub const GROUND: LayerId = LayerId::new(1);
}

fn main() {
    let mut nav_grid = NavGrid::new(64, 64);
    nav_grid.add_layer(nav_layer::GROUND);

    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(SimulationPlugin::new(
            GameSession::new(0, vec![PlayerSlot::occupied(0, PlayerType::Human)]),
            Map::new("demo", Projection::Isometric, nav_grid),
        ))
        .add_systems(Startup, setup)
        .add_systems(
            FixedUpdate,
            (print_entities, check_done).chain().after(SimulationSet),
        )
        .run();
}

fn setup(world: &mut World) {
    {
        let mut registry = world.resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("footman")
                .with_location(nav_layer::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.3))
                .with_health(60)
                .with_dying(5, None)
                .with_attack(10, 1, 3, 3),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(nav_layer::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_dying(4, None),
        );
    }

    let footman_pos = FixedUVec2::new(FixedU64::from_num(5u32), FixedU64::from_num(5u32));
    let (_, footman_id) = spawn::spawn_entity(world, "footman", footman_pos, Some(0))
        .expect("footman type must be registered");

    let dummy_pos = FixedUVec2::new(FixedU64::from_num(10u32), FixedU64::from_num(10u32));
    let (_, dummy_id) = spawn::spawn_entity(world, "dummy", dummy_pos, None)
        .expect("dummy type must be registered");

    world.resource_mut::<GameSession>().start();
    println!("spawned footman {footman_id:?} at (5, 5) and dummy {dummy_id:?} at (10, 10)");

    let mut pending = world.resource_mut::<PendingInput>();
    pending.push(PlayerCommand::SelectById { id: footman_id });
    pending.push(PlayerCommand::Attack {
        target: dummy_id,
        flush: true,
    });
}

fn print_entities(
    session: Res<GameSession>,
    query: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&HealthComponent>,
        Option<&DyingComponent>,
    )>,
) {
    let tick = session.tick();

    // Bevy query iteration order is not stable, so sort by the simulation id to
    // get a deterministic, reproducible printout.
    let mut entities: Vec<_> = query.iter().collect();
    entities.sort_unstable_by_key(|(info, ..)| info.id());

    println!("  tick {tick:>2}");

    for (info, loc, health, dying) in entities {
        let nav_pos = NavPos::from(loc.position);
        let hp = health.map_or(String::new(), |h| format!(", {} hp", h.current()));
        let state = if dying.is_some() { ", dying" } else { "" };
        println!(
            "   {:?} at ({}, {}){hp}{state}",
            info.id(),
            nav_pos.x,
            nav_pos.y
        );
    }
}

fn check_done(mut session: ResMut<GameSession>, mut exit: MessageWriter<AppExit>) {
    if session.tick() >= 80 {
        session.stop();
        println!("stopped at tick {}", session.tick());
        exit.write(AppExit::Success);
    }
}
