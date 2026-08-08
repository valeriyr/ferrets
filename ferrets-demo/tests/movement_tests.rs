//! Group movement on the real scenario map and content under the continuous
//! model: a packed group ordered to one point must finish its walks and come
//! to rest, not mill around the destination forever.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{SimulationPlugin, instantiate_scenario};
use ferrets_demo::{content::CONTENT, scenario};
use ferrets_geometry::{cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    components::{
        location::LocationComponent, order_queue::OrderQueueComponent,
        resource::ResourceSourceComponent,
    },
    content::registry::ContentRegistry,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn,
};

#[test]
fn continuous_group_move_settles_without_milling() {
    let mut app = scenario_app(MovementModel::Continuous);

    // The mission's own worker pair plus two more packed beside them, as
    // freshly trained units leave a building — a marching column whose
    // bodies arrive together and contest the exact destination.
    let mut units: Vec<Entity> = Vec::new();
    for (x, y) in [(10, 6), (10, 7)] {
        let (entity, _) = spawn::spawn_entity(
            app.world_mut(),
            "peasant",
            FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y)),
            Some(0),
        )
        .expect("extra peasant spawns");
        units.push(entity);
    }
    {
        let world = app.world_mut();
        let mission_pair: Vec<Entity> = world
            .query::<(Entity, &LocationComponent)>()
            .iter(world)
            .filter(|(_, location)| location.position.x == FixedU64::from_num(9))
            .map(|(entity, _)| entity)
            .collect();
        units.extend(mission_pair);
    }
    assert_eq!(units.len(), 4, "four peasants march");

    let target = FixedUVec2::new(FixedU64::from_num(20), FixedU64::from_num(20));
    for &unit in &units {
        app.world_mut()
            .entity_mut(unit)
            .get_mut::<OrderQueueComponent>()
            .unwrap()
            .push(
                Order::Move {
                    target,
                    size: CellSize::ONE,
                    range: 0,
                },
                None,
            );
    }

    utils::run_ticks(&mut app, 600);

    // Every walk must finish: the point is contested, so the losers accept a
    // ring around it instead of grinding at it forever.
    {
        let world = app.world_mut();
        for &unit in &units {
            assert!(
                world
                    .entity(unit)
                    .get::<OrderQueueComponent>()
                    .is_some_and(|queue| queue.0.is_empty()),
                "every walk into the crowd must finish"
            );
        }
    }

    // And finishing means rest: the settled pile must not churn bodies
    // around each other under sustained contact.
    let settled = positions(&mut app, &units);
    utils::run_ticks(&mut app, 50);
    assert_eq!(
        settled,
        positions(&mut app, &units),
        "a settled crowd must rest, not mill around"
    );

    // The exact settle, to the bit: each walk finishes the moment it
    // touches the ordered point, later arrivals shove earlier ones off it,
    // and the resting pile packs the block around it (sorted by position,
    // since the query order of the mission pair is not part of the
    // contract).
    let mut resting = settled.clone();
    resting.sort_unstable_by_key(|position| (position.x, position.y));
    assert_eq!(
        resting,
        vec![
            utils::position_bits(0x13_bd9e_c68b, 0x13_df97_b67b),
            utils::position_bits(0x14_a221_bf4f, 0x14_52ff_89b4),
            utils::position_bits(0x14_d1e9_68ec, 0x15_4e7f_f596),
            utils::position_bits(0x15_82be_54fe, 0x13_d82c_bb59),
        ]
    );

    // Rest also means separated circle bodies: no pair closer than a
    // body diameter.
    for (index, a) in settled.iter().enumerate() {
        for b in settled.iter().skip(index + 1) {
            let separation = a.distance(*b);
            assert!(
                separation >= FixedU64::from_num(0.99),
                "settled bodies must not overlap: separation {separation}"
            );
        }
    }
}

#[test]
fn harvest_of_unreachable_tree_gives_up() {
    let mut app = scenario_app(MovementModel::Continuous);

    // A tree fully ringed by other trees: no cell within harvest range is
    // passable, so the chase can never arrive. The order must finish — a
    // continuous mover's position wobbles by bits under pushing, and a
    // chase that compared exact positions instead of cells re-walked at
    // the grove's edge forever.
    let mut ring_target = None;
    for y in 20..23 {
        for x in 20..23 {
            let (tree, id) = spawn::spawn_entity(
                app.world_mut(),
                "tree",
                FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y)),
                None,
            )
            .expect("tree spawns");
            // A spawned source starts empty; stock it so the harvest
            // settles on the ordered tree instead of falling back to the
            // mission's grove.
            app.world_mut()
                .entity_mut(tree)
                .get_mut::<ResourceSourceComponent>()
                .unwrap()
                .amount = 400;
            if (x, y) == (21, 21) {
                ring_target = Some(id);
            }
        }
    }
    let (worker, _) = spawn::spawn_entity(
        app.world_mut(),
        "peasant",
        FixedUVec2::new(FixedU64::from_num(15), FixedU64::from_num(21)),
        Some(0),
    )
    .expect("worker spawns");

    app.world_mut()
        .entity_mut(worker)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Harvest {
                target: ring_target.unwrap(),
            },
            None,
        );

    utils::run_ticks(&mut app, 400);

    assert!(
        app.world()
            .entity(worker)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "a harvest whose target cannot be reached must give up"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// A headless single-player game on the built-in mission's map and content,
/// forced onto the given movement model.
fn scenario_app(model: MovementModel) -> App {
    let slots = vec![PlayerSlot::occupied(
        0,
        PlayerType::Human,
        Some("human"),
        None,
    )];
    let mission = scenario::builtin_mission(Projection::Isometric, model);
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let game_map = Map::from_data(&mission.map, &registry);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            "build_army",
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        game_map,
    ));
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() = registry;
        instantiate_scenario(world, &mission);
        world.resource_mut::<GameSession>().start();
    }
    app
}

fn positions(app: &mut App, units: &[Entity]) -> Vec<FixedUVec2> {
    units
        .iter()
        .map(|&unit| {
            app.world()
                .entity(unit)
                .get::<LocationComponent>()
                .unwrap()
                .position
        })
        .collect()
}
