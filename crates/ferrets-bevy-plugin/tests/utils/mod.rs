#![allow(dead_code)]

use bevy::ecs::entity::EntityNotSpawnedError;
use bevy::prelude::*;
use ferrets_bevy_plugin::{PendingInput, SimulationPlugin};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    astar::Projection,
    nav_grid::{LayerId, NavGrid},
    nav_pos::NavPos,
    nav_size::NavSize,
};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        entity_info::EntityInfoComponent,
        location::{LocationComponent, Solidity},
        order_queue::OrderQueueComponent,
        resource::{DepletionPolicy, HarvestData, HarvestVisibility},
    },
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    input::{InputFrames, PlayerFrame},
    map::Map,
    resources::PlayerResources,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, player_slot::PlayerSlot, player_type::PlayerType,
    },
};

pub const GROUND: LayerId = LayerId::new(1);

/// Creates an app with the simulation plugin on a 32×32 single-layer map,
/// with player slot `0` as the local player.
///
/// The session uses [`FinishPolicy::Endless`] so a lone or unpopulated slot is
/// never read as a win; a test that exercises the victory condition opts into
/// [`FinishPolicy::LastStanding`] with `set_finish_policy`. The caller registers
/// content and starts the session.
pub fn make_app(slots: Vec<PlayerSlot>) -> App {
    let mut nav_grid = NavGrid::new(32, 32);
    nav_grid.add_layer(GROUND);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            "test",
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        Map::new("test", Projection::Isometric, nav_grid, vec![]),
    ));
    app
}

pub fn pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

pub fn push_command(app: &mut App, command: PlayerCommand) {
    app.world_mut().resource_mut::<PendingInput>().push(command);
}

/// Runs exactly `ticks` fixed updates.
///
/// The simulation is deterministic, so tests advance by a known tick count and
/// then assert the resulting state, rather than polling for a condition.
///
/// Non-local players have no network peer in tests, so an idle frame is recorded
/// for them every tick — otherwise lockstep would block waiting for their commands.
pub fn run_ticks(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        let world = app.world_mut();

        let (current_tick, local_player, players) = {
            let session = world.resource::<GameSession>();
            let players: Vec<_> = session.slots().iter().map(|slot| slot.id()).collect();
            (session.tick(), session.local_player(), players)
        };
        for player in players {
            if player != local_player {
                world
                    .resource_mut::<InputFrames>()
                    .push_frame(PlayerFrame::idle(player, current_tick));
            }
        }

        world.run_schedule(FixedUpdate);
    }
}

/// Runs exactly `steps` fixed updates without synthesizing any input frames —
/// for suites whose registered frame sources already feed every slot.
pub fn run_steps(app: &mut App, steps: u32) {
    for _ in 0..steps {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

pub fn order_queue_is_empty(world: &mut World, entity: Entity) -> bool {
    world
        .get::<OrderQueueComponent>(entity)
        .is_some_and(|q| q.front().is_none())
}

/// Asserts `entity` has been despawned: looking it up fails specifically because
/// its id is now invalid (its slot was freed and the generation bumped), naming
/// exactly that entity — not merely with some error.
pub fn assert_despawned(world: &mut World, entity: Entity) {
    match world.get_entity(entity) {
        Ok(_) => panic!("expected {entity:?} to be despawned, but it is still alive"),
        Err(EntityNotSpawnedError::Invalid(error)) => assert_eq!(error.entity, entity),
        Err(other) => panic!("expected {entity:?} despawned (invalid id), got {other:?}"),
    }
}

/// The cell the entity currently stands on.
pub fn cell_of(world: &mut World, entity: Entity) -> NavPos {
    NavPos::from(world.get::<LocationComponent>(entity).unwrap().position)
}

/// Chebyshev distance — maximum of horizontal and vertical distances.
pub fn chebyshev(a: NavPos, b: NavPos) -> u32 {
    a.x.abs_diff(b.x).max(a.y.abs_diff(b.y))
}

/// Counts the entities of the given content type in the world.
pub fn count_of_type(world: &mut World, type_name: &str) -> usize {
    world
        .query::<&EntityInfoComponent>()
        .iter(world)
        .filter(|info| info.type_name() == type_name)
        .count()
}

/// Player 0's stockpile of gold.
pub fn gold(world: &World) -> u32 {
    world.resource::<PlayerResources>().amount(0, "gold")
}

/// App with the combat content roster — an attacking soldier (50 hp, 3-tick
/// dying phase) and an immobile dummy that leaves decaying bones — one human
/// player, session started.
pub fn combat_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(50)
                .with_dying(3, None)
                .with_attack(10, 1, 2, 2),
        );
        // Registered before `dummy`, which leaves it as a corpse.
        registry.register(
            EntityTypeDef::new("bones")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(3, Some("bones")),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();

    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// App with the economy content roster — soldier, worker, lumberjack, barracks,
/// depot, the gold/wood sources, and a passable ghost — two human players,
/// session started. Used by the order suites that exercise production, harvest,
/// movement, follow, and command dispatch.
pub fn orders_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    register_orders_content(&mut app);
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Registers the economy content roster ([`orders_app`]'s) and validates it.
pub fn register_orders_content(app: &mut App) {
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_resource("wood");
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 2, 2)
                .with_cost([("gold", 30)])
                .with_train_time(4),
        );
        // Registered before `worker`, which builds it.
        registry.register(
            EntityTypeDef::new("depot")
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 50)])
                .with_build_time(6)
                .with_resource_storage(["gold", "wood"]),
        );
        registry.register(
            EntityTypeDef::new("worker")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(2)
                .with_builder(["depot"])
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(5, 2, HarvestVisibility::Hidden),
                )]),
        );
        registry.register(
            EntityTypeDef::new("lumberjack")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_resource_carrier([(
                    "wood",
                    HarvestData::new(5, 2, HarvestVisibility::Visible),
                )]),
        );
        registry.register(
            EntityTypeDef::new("barracks")
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 40)])
                .with_build_time(4)
                .with_trainer(["soldier"]),
        );
        registry.register(
            EntityTypeDef::new("mine")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("tree")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("wood", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("geyser")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            EntityTypeDef::new("ghost")
                .with_location(GROUND, NavSize::ONE, Solidity::Passable)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
}
