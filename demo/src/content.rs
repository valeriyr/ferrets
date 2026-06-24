//! Demo content: two races (human, orc) plus neutral resource sources.
//!
//! Times are in ticks (20 Hz), tuned short so mechanics are quick to test.

use bevy::prelude::*;
use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::{
    components::{
        location::Solidity,
        resource::{DepletionPolicy, HarvestData, HarvestVisibility},
    },
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
};

use crate::map::GROUND;

const SPEED: f32 = 0.3;

/// Registers every race, resource kind, and entity type, then validates the
/// production catalogues. Runs at startup.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    registry.register_race("human");
    registry.register_race("orc");
    registry.register_resource("gold");
    registry.register_resource("wood");

    register_neutral(&mut registry);
    register_human(&mut registry);
    register_orc(&mut registry);

    registry.validate();
}

fn register_neutral(registry: &mut ContentRegistry) {
    registry.register(
        EntityTypeDef::new("gold_mine")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_resource_source("gold", DepletionPolicy::Persist),
    );
    registry.register(
        EntityTypeDef::new("tree")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_resource_source("wood", DepletionPolicy::Destroy),
    );
}

fn worker(name: &str, builds: [&str; 2]) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND, NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_num(SPEED))
        .with_health(30)
        .with_dying(2, None)
        .with_cost([("gold", 50)])
        .with_train_time(40)
        .with_builder(builds)
        .with_resource_carrier([
            ("gold", HarvestData::new(5, 20, HarvestVisibility::Hidden)),
            ("wood", HarvestData::new(5, 20, HarvestVisibility::Visible)),
        ])
}

fn main_hall(name: &str, worker_name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND, NavSize::new(3, 3), Solidity::Solid)
        .with_health(800)
        .with_dying(2, None)
        .with_cost([("gold", 400)])
        .with_build_time(200)
        .with_trainer([worker_name])
        .with_resource_storage(["gold", "wood"])
}

fn barracks(name: &str, unit_name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND, NavSize::new(3, 3), Solidity::Solid)
        .with_health(500)
        .with_dying(2, None)
        .with_cost([("gold", 200), ("wood", 100)])
        .with_build_time(120)
        .with_trainer([unit_name])
}

fn register_human(registry: &mut ContentRegistry) {
    registry.register(worker("peasant", ["town_hall", "barracks"]).with_race("human"));
    registry.register(main_hall("town_hall", "peasant").with_race("human"));
    registry.register(barracks("barracks", "archer").with_race("human"));
    // Ranged unit.
    registry.register(
        EntityTypeDef::new("archer")
            .with_race("human")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::from_num(SPEED))
            .with_health(40)
            .with_dying(2, None)
            .with_attack(6, 4, 3, 4)
            .with_cost([("gold", 80)])
            .with_train_time(60),
    );
}

fn register_orc(registry: &mut ContentRegistry) {
    registry.register(worker("peon", ["great_hall", "orc_barracks"]).with_race("orc"));
    registry.register(main_hall("great_hall", "peon").with_race("orc"));
    registry.register(barracks("orc_barracks", "grunt").with_race("orc"));
    // Melee unit.
    registry.register(
        EntityTypeDef::new("grunt")
            .with_race("orc")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::from_num(SPEED))
            .with_health(60)
            .with_dying(2, None)
            .with_attack(10, 1, 3, 3)
            .with_cost([("gold", 90)])
            .with_train_time(70),
    );
}
