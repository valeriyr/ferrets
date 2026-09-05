//! The demo's embedded AI script: it loads against the demo content and, in a
//! headless game, builds its economy and army.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{SimulationPlugin, ai::AiPlugin};
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::{
    ai::{conclave_ai, human_ai, install_demo_ai, orc_ai, swarm_ai},
    content::CONTENT,
    map, setup,
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::{
    ai::view::content::ContentView,
    content,
    engine::{ScriptEngine, lua::LuaEngine},
};
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, location::LocationComponent, owner::OwnerComponent,
    },
    map::Map,
    movement_model::MovementModel,
    player_research::PlayerResearch,
    session::{
        GameSession, ai_hosting::AiHosting, ai_vision::AiVision, authority::Authority,
        drop_policy::DropPolicy, finish_policy::FinishPolicy, local_role::LocalRole,
        player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn,
};

#[test]
fn ai_scripts_load() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let content = ContentView::from_registry(&registry);

    for script in [human_ai(), orc_ai(), swarm_ai(), conclave_ai()] {
        let runtime = LuaEngine.load_ai(&script, &content).expect("demo ai loads");
        assert_eq!(runtime.period(), 20);
    }
}

#[test]
fn field_races_ai_build_economy_and_army() {
    let slots = vec![
        // An idle human for the waves to march on.
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("swarm"),
            Some(1),
        ),
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("conclave"),
            Some(1),
        ),
        PlayerSlot::free(3),
    ];
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(0),
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

    for _ in 0..7000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let world = app.world_mut();
    // The swarm's structures are drones that changed: the pit the swarmlings
    // come from, a nest for headroom, a tumor walking the creep out — and the
    // drone line is kept topped up behind them.
    assert!(count_owned(world, 1, "spawning_pit") >= 1);
    assert!(count_owned(world, 1, "brood_nest") >= 1);
    assert!(count_owned(world, 1, "tumor") >= 1);
    assert!(count_owned(world, 1, "swarmling") >= 1);
    assert!(count_owned(world, 1, "ravager") + count_owned(world, 1, "cocoon") >= 1);
    assert!(count_owned(world, 1, "drone") >= 3);
    // The conclave's structures warped in on their own after a probe placed
    // them: the gateway in the nexus's power, a pylon, the cannon.
    assert!(count_owned(world, 2, "gateway") >= 1);
    assert!(count_owned(world, 2, "pylon") >= 1);
    assert!(count_owned(world, 2, "photon_cannon") >= 1);
    assert!(count_owned(world, 2, "zealot") >= 1);
    assert_eq!(count_owned(world, 2, "probe"), 5);
}

#[test]
fn ai_builds_economy_and_army() {
    let slots = vec![
        // An idle human, so the allied AIs have something to march on — the
        // wave crossing the map is the traversal this test also verifies.
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("human"),
            Some(1),
        ),
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("orc"),
            Some(1),
        ),
        PlayerSlot::free(3),
    ];
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(0),
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

    // 350 seconds of game time — the 96×96 map's longer walks stretch the
    // opening: the worker lines are trained, the production buildings stand,
    // each race's tech is researched — the human forge and the mortars it
    // unlocks, the orc ritual and the shamans it unlocks — and soldiers are
    // mustering.
    for _ in 0..7000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let world = app.world_mut();
    assert_eq!(world.resource::<GameSession>().tick(), 7000);
    assert_eq!(count_owned(world, 1, "peasant"), 5);
    assert_eq!(count_owned(world, 2, "peon"), 5);
    assert!(count_owned(world, 1, "barracks") >= 1);
    assert!(count_owned(world, 2, "war_camp") >= 1);
    assert!(count_owned(world, 1, "blacksmith") >= 1);
    assert!(count_owned(world, 1, "archer") >= 1);
    assert!(count_owned(world, 1, "mortar") >= 1);
    assert!(count_owned(world, 2, "grunt") >= 1);
    assert!(count_owned(world, 2, "shaman") >= 1);
    // The orc siege line: the works that requires the camp, and the pair of
    // wagons it trains — the demo's turreted mover.
    assert_eq!(count_owned(world, 2, "siege_works"), 1);
    assert_eq!(count_owned(world, 2, "war_wagon"), 2);
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

    // The mustered wave marched on the idle player across the map — through
    // a river ford or around the lake — so the army traverses the large map
    // instead of stalling at the chokepoints.
    let crossed = world
        .query::<(&EntityInfoComponent, &OwnerComponent, &LocationComponent)>()
        .iter(world)
        .any(|(info, owner, location)| {
            owner.player() == 1
                && info.type_name() == "archer"
                && location.position.x < FixedU64::from_num(48)
        });
    assert!(crossed, "the wave must cross the map's rivers");
}

#[test]
fn boss_mans_its_fleet_and_defends_lake() {
    // One idle human plus the boss slot the demo map's fleet belongs to.
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::free(1),
        PlayerSlot::free(2),
        PlayerSlot::free(3),
        PlayerSlot::environment(map::BOSS, AiVision::Filtered),
    ];
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(0),
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
    utils::create_entity(
        app.world_mut(),
        "archer",
        FixedUVec2::new(FixedU64::from_num(40), FixedU64::from_num(53)),
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

#[test]
fn ai_economy_runs_under_continuous_movement() {
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("human"),
            Some(1),
        ),
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("orc"),
            Some(1),
        ),
        PlayerSlot::free(3),
    ];
    let mut data = map::data();
    data.set_movement_model(MovementModel::Continuous);
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let game_map = Map::from_data(&data, &registry);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(0),
            slots,
            map::NAME,
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        game_map,
    ));
    app.add_plugins(AiPlugin);
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() =
            content::load(&LuaEngine, CONTENT).expect("demo content");
        setup::spawn_demo_scene(world);
        install_demo_ai(world);
    }

    for _ in 0..5000 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    // The same opening plays out on free positions and pushing: the worker
    // lines are trained, the production buildings stand, and army units
    // muster.
    let world = app.world_mut();
    assert_eq!(count_owned(world, 1, "peasant"), 5);
    assert_eq!(count_owned(world, 2, "peon"), 5);
    assert!(count_owned(world, 1, "barracks") >= 1);
    assert!(count_owned(world, 2, "war_camp") >= 1);
    assert!(count_owned(world, 1, "archer") >= 1);
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
