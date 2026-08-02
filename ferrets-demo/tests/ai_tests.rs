//! The demo's embedded AI script: it loads against the demo content and, in a
//! headless game, builds its economy and army.

use bevy::prelude::*;
use ferrets_bevy_plugin::SimulationPlugin;
use ferrets_bevy_plugin::ai::AiPlugin;
use ferrets_demo::ai::{human_ai, install_demo_ai, orc_ai};
use ferrets_demo::content::CONTENT;
use ferrets_demo::{map, setup};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::content;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::components::entity_info::EntityInfoComponent;
use ferrets_simulation::components::owner::OwnerComponent;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::player_research::PlayerResearch;
use ferrets_simulation::session::{
    GameSession,
    ai_hosting::AiHosting,
    authority::Authority,
    drop_policy::DropPolicy,
    finish_policy::FinishPolicy,
    player_slot::{PlayerId, PlayerSlot},
    player_type::PlayerType,
};
use ferrets_simulation::spawn;

#[test]
fn ai_scripts_load() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let content = ContentView::from_registry(&registry);

    for script in [human_ai(), orc_ai()] {
        let runtime = LuaEngine.load_ai(&script, &content).expect("demo ai loads");
        assert_eq!(runtime.period(), 20);
    }
}

#[test]
fn ai_builds_economy_and_army() {
    let slots = vec![
        // An idle human, so the AIs have something to attack eventually.
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(1, PlayerType::Ai, Some("human"), None),
        PlayerSlot::occupied(2, PlayerType::Ai, Some("orc"), None),
        PlayerSlot::free(3),
    ];
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
    app.add_plugins(AiPlugin);
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() =
            content::load(&LuaEngine, CONTENT).expect("demo content");
        setup::spawn_demo_scene(world);
        install_demo_ai(world);
    }

    // 100 seconds of game time: the worker lines are trained, the production
    // buildings stand, each race's tech is researched — the human forge and
    // the mortars it unlocks, the orc ritual and the shamans it unlocks — and
    // soldiers are mustering.
    for _ in 0..2000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let world = app.world_mut();
    assert_eq!(world.resource::<GameSession>().tick(), 2000);
    assert_eq!(count_owned(world, 1, "peasant"), 5);
    assert_eq!(count_owned(world, 2, "peon"), 5);
    assert!(count_owned(world, 1, "barracks") >= 1);
    assert!(count_owned(world, 2, "war_camp") >= 1);
    assert!(count_owned(world, 1, "blacksmith") >= 1);
    assert!(count_owned(world, 1, "archer") >= 1);
    assert!(count_owned(world, 1, "mortar") >= 1);
    assert!(count_owned(world, 2, "grunt") >= 1);
    assert!(count_owned(world, 2, "shaman") >= 1);
    for (player, research) in [(1, "iron_weapons"), (2, "frenzy_ritual")] {
        let id = world
            .resource::<ContentRegistry>()
            .research(research)
            .expect("research defined");
        assert!(
            world.resource::<PlayerResearch>().is_completed(player, id),
            "player {player} researched {research}"
        );
    }
}

#[test]
fn boss_mans_its_fleet_and_defends_the_lake() {
    // One idle human plus the boss slot the demo map's fleet belongs to.
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::free(1),
        PlayerSlot::free(2),
        PlayerSlot::free(3),
        PlayerSlot::environment(map::BOSS),
    ];
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
    app.add_plugins(AiPlugin);
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() =
            content::load(&LuaEngine, CONTENT).expect("demo content");
        setup::spawn_demo_scene(world);
        install_demo_ai(world);
    }

    // A lone archer strays to the lake shore, within a ship's aggro range.
    spawn::spawn_entity(
        app.world_mut(),
        "archer",
        FixedUVec2::new(FixedU64::from_num(26), FixedU64::from_num(38)),
        Some(0),
    )
    .expect("shore archer");

    for _ in 0..1000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let world = app.world_mut();
    // The ships shelled the stray archer, and the fortress trained the fleet
    // up to the brain's cap of four — ships are free, so the boss's empty
    // stockpile never blocks production.
    assert_eq!(count_owned(world, 0, "archer"), 0);
    assert_eq!(count_owned(world, map::BOSS, "ship"), 4);
    assert_eq!(count_owned(world, map::BOSS, "sea_fortress"), 1);
    // The boss neither ends the game nor gets eliminated.
    let session = world.resource::<GameSession>();
    assert_eq!(session.result(), None);
    assert!(!session.is_player_eliminated(map::BOSS));

    // Two ships sink; the fortress rebuilds the fleet — free production keeps
    // running, not just the opening batch.
    let sunk: Vec<Entity> = world
        .query::<(Entity, &EntityInfoComponent, &OwnerComponent)>()
        .iter(world)
        .filter(|(_, info, owner)| info.type_name() == "ship" && owner.player() == map::BOSS)
        .map(|(entity, _, _)| entity)
        .take(2)
        .collect();
    for ship in sunk {
        spawn::destroy_entity(world, ship);
    }
    for _ in 0..600 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    assert_eq!(count_owned(app.world_mut(), map::BOSS, "ship"), 4);
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
