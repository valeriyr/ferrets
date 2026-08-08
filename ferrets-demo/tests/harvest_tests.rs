//! Harvesting on the real demo map: the grove, the walk, and the drop-off
//! all work against the shipping map and content.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::SimulationPlugin;
use ferrets_demo::{content::CONTENT, map, setup};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, location::LocationComponent,
        order_queue::OrderQueueComponent,
    },
    content::registry::ContentRegistry,
    order::Order,
    resources::PlayerResources,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn worker_harvests_wood_on_demo_map() {
    let slots = vec![PlayerSlot::occupied(
        0,
        PlayerType::Human,
        Some("human"),
        None,
    )];
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            map::NAME,
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        map::build(),
    ));
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() =
            content::load(&LuaEngine, CONTENT).expect("demo content");
        setup::spawn_demo_scene(world);
        world.resource_mut::<GameSession>().start();
    }

    // A worker beside player 0's grove at (8..10, 16..17).
    let (worker, worker_id) = spawn::spawn_entity(
        app.world_mut(),
        "peasant",
        FixedUVec2::new(FixedU64::from_num(7), FixedU64::from_num(16)),
        Some(0),
    )
    .expect("worker spawns");

    // Find a tree's id to harvest.
    let tree_id: SimulationId = {
        let world = app.world_mut();
        world
            .query::<(&EntityInfoComponent, &LocationComponent)>()
            .iter(world)
            .find(|(info, location)| {
                info.type_name() == "tree" && location.position.x == FixedU64::from_num(8)
            })
            .map(|(info, _)| info.id())
            .expect("tree exists")
    };

    let _ = worker_id;
    app.world_mut()
        .entity_mut(worker)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(Order::Harvest { target: tree_id }, None);

    utils::run_ticks(&mut app, 300);

    // Exact values — the harvest cycle is deterministic lockstep math, so
    // the trip count and the worker's resting spot replay identically on
    // every peer; a drifted amount is a desync, not a tuning detail.
    let world = app.world_mut();
    let wood = world.resource::<PlayerResources>().amount(0, "wood");
    assert_eq!(wood, 235, "wood banked by tick 300");
    assert_eq!(
        world
            .entity(worker)
            .get::<LocationComponent>()
            .unwrap()
            .position,
        FixedUVec2::new(FixedU64::from_num(9), FixedU64::from_num(15)),
        "the worker rests on the lattice beside its work"
    );
}
