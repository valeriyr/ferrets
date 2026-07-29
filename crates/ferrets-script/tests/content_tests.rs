//! Loading content into a `ContentRegistry`: the definitions round-trip to
//! the same `EntityTypeDef`s the Rust builder produces, and malformed scripts
//! surface as errors rather than panics. The contract holds for any engine;
//! [`engine`] picks the binding the suite runs against.

use ferrets_math::FixedU64;
use ferrets_pathfinder::{nav_grid::LayerId, nav_size::NavSize};
use ferrets_script::content;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_script::error::ScriptError;
use ferrets_simulation::content::{
    entity_type_def::EntityTypeDef,
    location::Solidity,
    skills::{SkillDef, SkillEffect, SkillTarget},
    splash::SplashShape,
    stats::StatId,
};

//
// ─── Round-trip ─────────────────────────────────────────────────────────────
//

#[test]
fn loads_races_resources_and_entities() {
    let registry = content::load(&engine(), ARCHER).expect("load content");

    assert!(registry.has_race("human"));
    assert!(registry.has_resource("gold"));
    assert!(registry.has_layer("ground"));

    let expected = EntityTypeDef::new("archer")
        .with_race("human")
        .with_location(LayerId::new(1), NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_str("0.3").unwrap())
        .with_health(40)
        .with_dying(2, None)
        .with_attack(6, 4, 4, 7, 3)
        .with_cost([("gold", 80)])
        .with_train_time(60);

    assert_eq!(registry.entity("archer"), Some(&expected));
}

#[test]
fn declared_acquire_range_overrides_weapon_range_default() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("scout", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 20,
                damage = 2, attack_range = 3, acquire_range = 7, attack_period = 4, damage_point = 2,
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("scout")
        .with_location(LayerId::new(1), NavSize::ONE, Solidity::Solid)
        .with_health(20)
        .with_attack(2, 3, 7, 4, 2);

    assert_eq!(registry.entity("scout"), Some(&expected));
}

#[test]
fn custom_stat_is_declared_and_seeded() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_stat("morale")
        define_entity("hero", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10, morale = 7 },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let morale = registry.stat("morale").expect("morale is registered");
    assert_eq!(
        registry.entity("hero").unwrap().base_stat(morale),
        Some(FixedU64::from_num(7)),
    );
}

#[test]
fn unknown_stat_name_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("gadget", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { bogus = 1 },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown stat");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("stat 'bogus' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn parses_armor_bonus_damage_vs_and_energy() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("knight", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100,
                damage = 12, attack_range = 1, attack_period = 4, damage_point = 2,
                armor = 4, max_energy = 50, energy_regen = "0.5",
            },
            bonus_damage_vs = { armored = 8, dragon = 15 },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("knight")
        .with_location(LayerId::new(1), NavSize::ONE, Solidity::Solid)
        .with_health(100)
        .with_attack(12, 1, 1, 4, 2)
        .with_armor(4)
        .with_bonus_damage_vs([("armored", 8u32), ("dragon", 15u32)])
        .with_energy(50, FixedU64::from_str("0.5").unwrap());

    assert_eq!(registry.entity("knight"), Some(&expected));
}

#[test]
fn parses_skill_with_buff_effect() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_buff("haste", {
            duration = 20,
            stack = "refresh",
            modifiers = {
                { stat = "damage", op = "percent", value = "1.0" },
            },
        })

        define_skill("battle_focus", {
            cooldown = 5,
            energy_cost = "30",
            target = "self",
            effect = { apply_buff = "haste" },
        })

        define_entity("mage", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 40, max_energy = 100, energy_regen = "1" },
            skills = { "battle_focus" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let mage = registry.entity("mage").expect("mage defined");
    let haste = registry.buff("haste").expect("haste buff defined");
    let battle_focus = registry.skill("battle_focus").expect("skill defined");

    // The entity references the skill by id, and the registered definition carries
    // the parsed cooldown, cost, and effect.
    assert_eq!(mage.skills, vec![battle_focus]);
    assert_eq!(
        registry.skill_def(battle_focus),
        &SkillDef {
            cooldown: 5,
            energy_cost: FixedU64::from_num(30),
            target: SkillTarget::Caster,
            effect: SkillEffect::ApplyBuff(haste),
        }
    );
}

#[test]
fn parses_projectile_and_splash() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_projectile("shell", { speed = "0.4", aim = "position" })

        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 30,
                damage = 12, attack_range = 6, attack_period = 10, damage_point = 4,
            },
            projectile = "shell",
            splash = {
                shape = "circular",
                bands = { {1, "0.5"}, {2, "0.25"} },
                layers = GROUND,
                friendly_fire = true,
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("mortar")
        .with_location(LayerId::new(1), NavSize::ONE, Solidity::Solid)
        .with_health(30)
        .with_attack(12, 6, 6, 10, 4)
        .with_projectile(registry.projectile("shell").expect("shell is registered"))
        .with_splash(
            SplashShape::Circular,
            vec![
                (1, FixedU64::from_str("0.5").unwrap()),
                (2, FixedU64::from_str("0.25").unwrap()),
            ],
            LayerId::new(1),
            true,
        );

    assert_eq!(registry.entity("mortar"), Some(&expected));
}

#[test]
fn splash_rejects_missing_fields() {
    // Splash has no engine defaults, so omitting any field is a content error rather
    // than a silent fallback.
    for (missing, block) in [
        (
            "shape",
            r#"splash = { bands = { {1, "0.5"} }, layers = GROUND, friendly_fire = false },"#,
        ),
        (
            "bands",
            r#"splash = { shape = "circular", layers = GROUND, friendly_fire = false },"#,
        ),
        (
            "layers",
            r#"splash = { shape = "circular", bands = { {1, "0.5"} }, friendly_fire = false },"#,
        ),
        (
            "friendly_fire",
            r#"splash = { shape = "circular", bands = { {1, "0.5"} }, layers = GROUND },"#,
        ),
    ] {
        let Err(error) = content::load(&engine(), &attacker_with(block)) else {
            panic!("'{missing}' must be required");
        };
        assert!(
            error.to_string().contains(missing),
            "the error must name the missing '{missing}' field, got: {error}"
        );
    }
}

#[test]
fn unknown_projectile_is_rejected() {
    let Err(error) = content::load(&engine(), &attacker_with(r#"projectile = "boulder","#)) else {
        panic!("an unregistered projectile must be rejected");
    };
    assert_eq!(
        error.to_string(),
        "content error: projectile 'boulder' is not defined"
    );
}

#[test]
fn define_projectile_rejects_missing_fields() {
    // Neither field has a default: a speed is the flight time and an aim decides
    // whether the hit follows its target or lands on a cell.
    for (missing, block) in [("speed", r#"aim = "entity""#), ("aim", r#"speed = "0.4""#)] {
        let source = format!(r#"define_projectile("arrow", {{ {block} }})"#);
        let Err(error) = content::load(&engine(), &source) else {
            panic!("'{missing}' must be required");
        };
        assert!(
            error.to_string().contains(missing),
            "the error must name the missing '{missing}' field, got: {error}"
        );
    }
}

#[test]
fn unknown_projectile_aim_is_rejected() {
    let source = r#"
        define_projectile("arrow", { speed = "0.4", aim = "sideways" })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("an unknown aim must be rejected");
    };
    assert_eq!(
        error.to_string(),
        "content error: unknown attack aim 'sideways'"
    );
}

#[test]
fn parses_selection_priority_and_class() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("caster", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, sight_range = 12 },
            selection = { priority = 42, class = "spellcaster" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("caster")
        .with_location(LayerId::new(1), NavSize::ONE, Solidity::Solid)
        .with_health(20)
        .with_selection(42, Some("spellcaster"))
        .with_sight_range(12);

    assert_eq!(registry.entity("caster"), Some(&expected));
}

#[test]
fn selection_class_defaults_to_type_name() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("marine", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let marine = registry.entity("marine").expect("marine");
    assert_eq!(marine.selection_class(), "marine");
    assert_eq!(marine.selection.priority(), 0);
    assert_eq!(marine.base_stat(StatId::SIGHT_RANGE), None);
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
// ─── Tags ─────────────────────────────────────────────────────────────────────
//

#[test]
fn loads_declared_tags_onto_entities() {
    let source = r#"
        local ground = define_layer("ground")
        define_tag("flying")
        define_entity("gryphon", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            tags = { "flying" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert!(registry.has_tag("flying"));
    assert!(registry.entity("gryphon").unwrap().tags.contains("flying"));
}

#[test]
#[should_panic(expected = "references unregistered tag 'flying'")]
fn undeclared_tag_panics_on_load() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("gryphon", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            tags = { "flying" },
        })
    "#;
    let _ = content::load(&engine(), source);
}

//
// ─── Layers ───────────────────────────────────────────────────────────────────
//

#[test]
fn define_layer_returns_ids_in_declaration_order() {
    let source = r#"
        local ground = define_layer("ground")
        local air = define_layer("air")
        assert(ground == 1)
        assert(air == 2)
        assert(define_layer("ground") == 1)
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert!(registry.has_layer("ground"));
    assert!(registry.has_layer("air"));
}

#[test]
fn loads_occupation_of_several_layers_combined_with_bitwise_or() {
    let source = r#"
        local ground = define_layer("ground")
        local air = define_layer("air")
        define_entity("gryphon", {
            location = { occupation = ground | air, size = 1, solidity = "solid" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("gryphon").with_location(
        LayerId::new(1) | LayerId::new(2),
        NavSize::ONE,
        Solidity::Solid,
    );
    assert_eq!(registry.entity("gryphon"), Some(&expected));
}

#[test]
fn layer_id_looks_up_defined_layer() {
    let source = r#"
        define_layer("ground")
        define_layer("air")
        define_entity("gryphon", {
            location = { occupation = layer_id("air"), size = 1, solidity = "solid" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected =
        EntityTypeDef::new("gryphon").with_location(LayerId::new(2), NavSize::ONE, Solidity::Solid);
    assert_eq!(registry.entity("gryphon"), Some(&expected));
}

#[test]
fn reports_undefined_layer_lookup_as_content_error() {
    let source = r#"
        define_entity("gryphon", {
            location = { occupation = layer_id("ground"), size = 1, solidity = "solid" },
        })
    "#;

    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject undefined layer lookup");
    };

    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("layer 'ground' is not defined")),
        "got {error:?}"
    );
}

//
// ─── Terrains ─────────────────────────────────────────────────────────────────
//

#[test]
fn loads_declared_terrains() {
    let source = r#"
        local ground = define_layer("ground")
        local water = define_layer("water")
        define_terrain("grass", ground)
        define_terrain("shore", ground | water)
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert_eq!(
        registry.terrain("grass"),
        Some(LayerId::new(1).into()),
        "grass passes ground only"
    );
    assert_eq!(
        registry.terrain("shore"),
        Some(LayerId::new(1) | LayerId::new(2)),
        "shore passes both layers"
    );
}

#[test]
#[should_panic(expected = "terrain 'water' passes unregistered layers")]
fn terrain_passing_undeclared_layer_panics_on_load() {
    let source = r#"
        define_layer("ground")
        define_terrain("water", 2)
    "#;
    let _ = content::load(&engine(), source);
}

#[test]
#[should_panic(expected = "entity type 'gryphon' occupies unregistered layers")]
fn undeclared_occupation_layer_panics_on_load() {
    let source = r#"
        define_layer("ground")
        define_entity("gryphon", {
            location = { occupation = 8, size = 1, solidity = "solid" },
        })
    "#;
    let _ = content::load(&engine(), source);
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
            stats = { speed = "fast" },
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
fn reports_error_catching_as_engine_error() {
    // A content script must not swallow a failed declaration: catching the
    // error would let the load succeed with the definition silently missing.
    for source in [
        r#"pcall(define_tag, "")"#,
        r#"xpcall(define_tag, function() end, "")"#,
    ] {
        let Err(error) = content::load(&engine(), source) else {
            panic!("must reject error catching: {source}");
        };

        assert!(
            matches!(&error, ScriptError::EngineError(m) if m.contains("error catching is unavailable")),
            "{source}: got {error:?}"
        );
    }
}

#[test]
fn rejects_catching_errors_through_coroutines() {
    // `coroutine.resume` returns a failed body as `(false, error)` instead of
    // raising, so the library as a whole is withdrawn from content scripts.
    let source = r#"coroutine.resume(coroutine.create(function() define_tag("") end))"#;

    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject error catching through a coroutine");
    };

    let ScriptError::EngineError(message) = &error else {
        panic!("expected an engine error, got {error:?}");
    };
    assert!(
        message.contains("coroutine"),
        "unexpected message: {message}"
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
    local GROUND = define_layer("ground")

    define_race("human")

    define_resource("gold")

    define_entity("archer", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", max_health = 40,
            damage = 6, attack_range = 4, attack_period = 7, damage_point = 3,
        },
        dying = { time = 2 },
        cost = { gold = 80 },
        train_time = 60,
    })
"#;

/// A worker and a hall referencing each other's catalogues.
const BASE: &str = r#"
    local GROUND = define_layer("ground")

    define_race("human")

    define_resource("gold")
    define_resource("wood")

    define_entity("peasant", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { speed = "0.3", max_health = 30 },
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
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800 },
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

/// An attacker table with `block` spliced in, for the delivery-field error cases.
fn attacker_with(block: &str) -> String {
    format!(
        r#"
        local GROUND = define_layer("ground")

        define_entity("mortar", {{
            location = {{ occupation = GROUND, size = 1, solidity = "solid" }},
            stats = {{ damage = 12, attack_range = 6, attack_period = 10, damage_point = 4 }},
            {block}
        }})
        "#
    )
}
