//! Loading content into a `ContentRegistry`: the definitions round-trip to
//! the same `EntityTypeDef`s the Rust builder produces, and malformed scripts
//! surface as errors rather than panics. The contract holds for any engine;
//! [`engine`] picks the binding the suite runs against.

use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_script::content;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_script::error::ScriptError;
use ferrets_simulation::components::location::Solidity;
use ferrets_simulation::content::entity_type_def::EntityTypeDef;

//
// ─── Round-trip ─────────────────────────────────────────────────────────────
//

#[test]
fn loads_races_resources_and_entities() {
    let registry = content::load(&engine(), ARCHER).expect("load content");

    assert!(registry.has_race("human"));
    assert!(registry.has_resource("gold"));

    let expected = EntityTypeDef::new("archer")
        .with_race("human")
        .with_location(1u32, NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_str("0.3").unwrap())
        .with_health(40)
        .with_dying(2, None)
        .with_attack(6, 4, 3, 4)
        .with_cost([("gold", 80)])
        .with_train_time(60);

    assert_eq!(registry.entity("archer"), Some(&expected));
}

#[test]
fn wires_production_catalogues_across_entities() {
    // A worker that builds a hall, and a hall that trains the worker — the cyclic
    // catalogue that only validates once both are registered.
    let registry = content::load(&engine(), BASE).expect("load content");

    let worker = registry.entity("peasant").expect("peasant");
    assert!(
        worker
            .builder
            .as_ref()
            .unwrap()
            .builds()
            .any(|b| b == "town_hall")
    );

    let hall = registry.entity("town_hall").expect("town_hall");
    assert!(
        hall.trainer
            .as_ref()
            .unwrap()
            .trains()
            .any(|t| t == "peasant")
    );
}

//
// ─── Errors, not panics ──────────────────────────────────────────────────────
//

#[test]
fn reports_unknown_enum_as_content_error() {
    let source = r#"
        define_entity("wall", {
            location = { occupation = 1, size = 1, solidity = "wobbly" },
        })
    "#;

    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject unknown solidity");
    };

    let ScriptError::ContentError(message) = &error else {
        panic!("expected a content error, got {error:?}");
    };
    assert!(
        message.contains("solidity") && message.contains("wobbly"),
        "unexpected message: {message}"
    );
}

#[test]
fn reports_malformed_number_as_number_error() {
    let source = r#"
        define_entity("archer", {
            location = { occupation = 1, size = 1, solidity = "solid" },
            movement = { speed = "fast" },
        })
    "#;

    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject malformed number");
    };

    let ScriptError::NumberError(message) = &error else {
        panic!("expected a number error, got {error:?}");
    };
    assert!(message.contains("fast"), "unexpected message: {message}");
}

#[test]
fn reports_ambient_state_use_as_engine_error() {
    let source = r#"
        define_race("human")
        define_resource("gold")
        local roll = math.random(1, 6)
    "#;

    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject ambient randomness");
    };

    assert!(
        matches!(&error, ScriptError::EngineError(m) if m.contains("math.random is unavailable")),
        "got {error:?}"
    );
}

#[test]
fn reports_lua_syntax_error_as_engine_error() {
    let Err(error) = content::load(&engine(), "this is not valid lua ]]}}") else {
        panic!("must reject invalid lua");
    };

    let ScriptError::EngineError(message) = &error else {
        panic!("expected an engine error, got {error:?}");
    };
    assert!(message.contains("syntax"), "unexpected message: {message}");
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// One self-contained ranged unit (no production catalogue).
const ARCHER: &str = r#"
    define_race("human")

    define_resource("gold")

    define_entity("archer", {
        race = "human",
        location = { occupation = 1, size = 1, solidity = "solid" },
        movement = { speed = "0.3" },
        health = 40,
        dying = { time = 2 },
        attack = { damage = 6, range = 4, aiming = 3, reloading = 4 },
        cost = { gold = 80 },
        train_time = 60,
    })
"#;

/// A worker and a hall referencing each other's catalogues.
const BASE: &str = r#"
    define_race("human")

    define_resource("gold")
    define_resource("wood")

    define_entity("peasant", {
        race = "human",
        location = { occupation = 1, size = 1, solidity = "solid" },
        movement = { speed = "0.3" },
        health = 30,
        dying = { time = 2 },
        cost = { gold = 50 },
        train_time = 40,
        builder = { "town_hall" },
        resource_carrier = {
            gold = { capacity = 5, time = 20, visibility = "hidden" },
            wood = { capacity = 5, time = 20, visibility = "visible" },
        },
    })

    define_entity("town_hall", {
        race = "human",
        location = { occupation = 1, size = { 3, 3 }, solidity = "solid" },
        health = 800,
        dying = { time = 2 },
        cost = { gold = 400 },
        build_time = 200,
        trainer = { "peasant" },
        resource_storage = { "gold", "wood" },
    })
"#;

/// The engine the suite runs against — the only line naming a binding.
fn engine() -> impl ScriptEngine {
    LuaEngine
}
