//! Demo content: two races (human, orc) plus neutral resource sources, authored
//! in Lua and loaded at startup.
//!
//! Times are in ticks (20 Hz), tuned short so mechanics are quick to test.

use bevy::prelude::*;
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::content::registry::ContentRegistry;

/// The demo's content, as a Lua script. It declares the ground and water
/// navigation layers (named by [`crate::map::GROUND`] and
/// [`crate::map::WATER`]) and a terrain for each. Fractional stats are decimal
/// strings so they parse straight to fixed-point (no `f64`).
pub const CONTENT: &str = r#"
    local GROUND = define_layer("ground")
    local WATER = define_layer("water")

    define_terrain("grass", GROUND)
    define_terrain("water", WATER)

    define_race("human")
    define_race("orc")

    define_resource("gold")
    define_resource("wood")

    -- The lake boss: a raceless water fortress spawning free ships. Ships are
    -- ranged so they shell shore targets; the fortress is the boss's building.
    define_entity("ship", {
        location = { occupation = WATER, size = 1, solidity = "solid" },
        movement = { speed = "0.25" },
        health = 80,
        dying = { time = 2 },
        attack = { damage = 12, range = 5, acquire_range = 8, aiming = 4, reloading = 6 },
        train_time = 100,
    })
    define_entity("sea_fortress", {
        location = { occupation = WATER, size = { 3, 3 }, solidity = "solid" },
        health = 1500,
        dying = { time = 2 },
        trainer = { "ship" },
        tags = { "building" },
    })

    -- Neutral resource sources.
    define_entity("gold_mine", {
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        resource_source = { kind = "gold", depletion = "persist" },
    })
    define_entity("tree", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        resource_source = { kind = "wood", depletion = "destroy" },
    })

    local function worker(name, race, builds)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            movement = { speed = "0.3" },
            health = 30,
            dying = { time = 2 },
            cost = { gold = 50 },
            train_time = 40,
            builder = builds,
            resource_carrier = {
                gold = { capacity = 5, time = 20, visibility = "hidden" },
                wood = { capacity = 5, time = 20, visibility = "visible" },
            },
        })
    end

    local function main_hall(name, race, trains)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            health = 800,
            dying = { time = 2 },
            cost = { gold = 400 },
            build_time = 200,
            trainer = { trains },
            resource_storage = { "gold", "wood" },
            tags = { "building" },
        })
    end

    local function barracks(name, race, trains)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            health = 500,
            dying = { time = 2 },
            cost = { gold = 200, wood = 100 },
            build_time = 120,
            trainer = { trains },
            tags = { "building" },
        })
    end

    -- Human: worker, base, barracks, and a ranged unit.
    worker("peasant", "human", { "town_hall", "barracks" })
    main_hall("town_hall", "human", "peasant")
    barracks("barracks", "human", "archer")
    define_entity("archer", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        movement = { speed = "0.3" },
        health = 40,
        dying = { time = 2 },
        attack = { damage = 6, range = 4, acquire_range = 7, aiming = 3, reloading = 4 },
        cost = { gold = 80 },
        train_time = 60,
        -- Combat units lead a mixed selection over workers.
        selection_priority = 10,
    })

    -- Orc: worker, base, barracks, and a melee unit.
    worker("peon", "orc", { "great_hall", "orc_barracks" })
    main_hall("great_hall", "orc", "peon")
    barracks("orc_barracks", "orc", "grunt")
    define_entity("grunt", {
        race = "orc",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        movement = { speed = "0.3" },
        health = 60,
        dying = { time = 2 },
        attack = { damage = 10, range = 1, acquire_range = 5, aiming = 3, reloading = 3 },
        cost = { gold = 90 },
        train_time = 70,
        selection_priority = 10,
    })
"#;

/// Loads all demo content from Lua into the registry, then validates it. Runs at
/// startup; a content error is a bug in the script above, so it panics.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    *registry = content::load(&LuaEngine, CONTENT).expect("demo content must load");
}
