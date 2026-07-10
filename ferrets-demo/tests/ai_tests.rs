//! The demo's embedded AI script: it loads against the demo content and, in a
//! headless game, builds its economy and army.

use bevy::prelude::*;
use ferrets_bevy_plugin::SimulationPlugin;
use ferrets_bevy_plugin::ai::AiPlugin;
use ferrets_demo::ai::{AI_SCRIPT, install_demo_ai};
use ferrets_demo::content::CONTENT;
use ferrets_demo::{map, setup};
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::content;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::components::entity_info::EntityInfoComponent;
use ferrets_simulation::components::owner::OwnerComponent;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::session::{
    GameSession,
    ai_hosting::AiHosting,
    authority::Authority,
    drop_policy::DropPolicy,
    finish_policy::FinishPolicy,
    player_slot::{PlayerId, PlayerSlot},
    player_type::PlayerType,
};

#[test]
fn ai_script_loads() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let content = ContentView::from_registry(&registry);

    let runtime = LuaEngine
        .load_ai(AI_SCRIPT, &content)
        .expect("demo ai loads");

    assert_eq!(runtime.period(), 20);
}

#[test]
fn ai_builds_economy_and_army() {
    let slots = vec![
        // An idle human, so the AIs have something to attack eventually.
        PlayerSlot::occupied(0, PlayerType::Human, Some("human")),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human")),
        PlayerSlot::occupied(2, PlayerType::Ai, Some("orc")),
        PlayerSlot::free(3),
    ];
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        map::build(),
    ));
    app.add_plugins(AiPlugin);
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() =
            content::load(&LuaEngine, CONTENT).expect("demo content");
        setup::spawn_demo_scene(world);
        install_demo_ai(world);
    }

    // 50 seconds of game time: the worker line is trained, the barracks
    // stands, and soldiers are mustering — but the first attack wave (at 5
    // soldiers) has not marched yet, so nothing has died.
    for _ in 0..1000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let world = app.world_mut();
    assert_eq!(world.resource::<GameSession>().tick(), 1000);
    assert!(count_owned(world, 1, "barracks") >= 1);
    assert!(count_owned(world, 2, "orc_barracks") >= 1);
    assert_eq!(count_owned(world, 1, "peasant"), 5);
    assert_eq!(count_owned(world, 2, "peon"), 5);
    assert!(count_owned(world, 1, "archer") >= 1);
    assert!(count_owned(world, 2, "grunt") >= 1);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

fn count_owned(world: &mut World, player: PlayerId, type_name: &str) -> usize {
    world
        .query::<(&EntityInfoComponent, &OwnerComponent)>()
        .iter(world)
        .filter(|(info, owner)| info.type_name() == type_name && owner.player() == player)
        .count()
}
