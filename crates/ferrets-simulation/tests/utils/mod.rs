#![allow(dead_code)]

//! Helpers shared by the simulation's integration suites.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_type_def::EntityTypeDef, location::Solidity, pool::Pool, registry::ContentRegistry,
};
use ferrets_geometry::{cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::nav_grid::NavGrid;
use ferrets_simulation::{
    control_groups::ControlGroups,
    entity_index::EntityIndex,
    events::EventRecord,
    map::Map,
    movement_model::MovementModel,
    player_buffs::PlayerBuffs,
    player_stats::PlayerStats,
    resources::PlayerResources,
    ruleset::{RemainsLimit, Ruleset},
    selection::Selection,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, local_role::LocalRole, player_slot::PlayerSlot,
        player_type::PlayerType,
    },
    simulation_id::SimulationIdGenerator,
    spawn::{self, FieldReach},
};

/// The players a [`world`] seats.
const PLAYERS: usize = 1;

/// A world ready to spawn simulation entities: the built-in registry with a
/// ground layer and one `unit` type (one cell, solid, carrying `pools`), a
/// 16-cell map, one seated human, and the stores a spawn and its stat fold
/// read.
pub fn world(pools: impl IntoIterator<Item = Pool>) -> World {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let unit = pools.into_iter().fold(
        EntityTypeDef::new("unit").with_location(ground, CellSize::ONE, Solidity::Solid),
        EntityTypeDef::with_pool,
    );
    registry.register(unit);
    registry.validate();
    let mut nav_grid = NavGrid::new(16, 16);
    nav_grid.add_layer(ground);

    let mut world = World::new();
    world.insert_resource(registry);
    world.insert_resource(GameSession::configured(
        LocalRole::Player(0),
        vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)],
        "test",
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        FinishPolicy::Endless,
        Ruleset::new(RemainsLimit::Unbounded),
    ));
    world.insert_resource(Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        nav_grid,
        vec![],
        &[],
    ));
    world.insert_resource(Selection::new(PLAYERS));
    world.insert_resource(ControlGroups::new(PLAYERS));
    world.insert_resource(PlayerResources::new(PLAYERS));
    world.insert_resource(PlayerStats::new(PLAYERS));
    world.insert_resource(PlayerBuffs::new(PLAYERS));
    world.insert_resource(EntityIndex::default());
    world.insert_resource(SimulationIdGenerator::default());
    world.insert_resource(EventRecord::default());
    world
}

/// Spawns an unowned `unit` at cell `(x, y)` of a [`world`].
pub fn spawn_unit(world: &mut World, x: u32, y: u32) -> Entity {
    let position = FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y));
    spawn::create_entity(world, "unit", position, None, FieldReach::Initial)
        .expect("the world registers a unit")
        .0
}
