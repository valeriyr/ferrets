//! ferrets demo — simulation driven by a Bevy app.
//!
//! Demonstrates the ferrets-bevy integration:
//!   1. register content types in Startup
//!   2. spawn a unit by type name and position
//!   3. push a move command via PendingInput
//!   4. read sim state directly via Query in FixedUpdate
use bevy::prelude::*;
use ferrets_bevy::{PendingInput, SimulationPlugin, SimulationSet};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    astar::Projection, nav_grid::NavGrid, nav_pos::NavPos, nav_size::NavSize,
};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{entity_info::EntityInfoComponent, location::LocationComponent},
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    map::Map,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn::spawn_entity,
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
            (print_locations, check_done).chain().after(SimulationSet),
        )
        .run();
}

fn setup(world: &mut World) {
    world.resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("footman", nav_layer::GROUND, NavSize::ONE)
            .with_movement(FixedU64::from_num(0.3)),
    );

    let pos = FixedUVec2::new(FixedU64::from_num(5u32), FixedU64::from_num(5u32));
    let (entity, id) =
        spawn_entity(world, "footman", pos).expect("footman type must be registered");

    world.resource_mut::<GameSession>().start();
    println!("spawned {id:?} ({entity:?}) at (5, 5)");

    let mut pending = world.resource_mut::<PendingInput>();
    pending.push(PlayerCommand::SelectById { id });
    pending.push(PlayerCommand::Move {
        target: FixedUVec2::new(FixedU64::from_num(10u32), FixedU64::from_num(10u32)),
        flush: true,
    });
}

fn print_locations(
    session: Res<GameSession>,
    query: Query<(&EntityInfoComponent, &LocationComponent)>,
) {
    let tick = session.tick();
    for (info, loc) in &query {
        let nav_pos = NavPos::from(loc.position);
        println!(
            "  tick {tick:>2} — {:?} at ({}, {})",
            info.id(),
            nav_pos.x,
            nav_pos.y
        );
    }
}

fn check_done(mut session: ResMut<GameSession>, mut exit: MessageWriter<AppExit>) {
    if session.tick() >= 20 {
        session.stop();
        println!("stopped at tick {}", session.tick());
        exit.write(AppExit::Success);
    }
}
