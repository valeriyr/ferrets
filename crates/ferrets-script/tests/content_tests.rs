//! Loading content into a `ContentRegistry`: the definitions round-trip to
//! the same `EntityTypeDef`s the Rust builder produces, and malformed scripts
//! surface as errors rather than panics. The contract holds for any engine;
//! [`engine`] picks the binding the suite runs against.

use ferrets_content::{
    costs,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime},
    player_stats::PlayerStatId,
    repair::{RepairCost, RepairRate},
    research::ResearchDef,
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillDef,
    },
    splash::SplashShape,
    stats::{EntityModifier, ModifierOp, PlayerModifier},
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    work::WorkPresence,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::nav_grid::LayerId;
use ferrets_script::{
    content,
    engine::{ScriptEngine, lua::LuaEngine},
    error::ScriptError,
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
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_movement(
            FixedU64::from_str("0.3").unwrap(),
            FixedU64::from_str("0.5").unwrap(),
        )
        .with_health(40)
        .with_dying(2, None)
        .with_attack(6, 4, 4, 7, 3)
        .with_targets(LayerId::new(1))
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
            targets = GROUND,
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("scout")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(20)
        .with_attack(2, 3, 7, 4, 2)
        .with_targets(LayerId::new(1));

    assert_eq!(registry.entity("scout"), Some(&expected));
}

#[test]
fn custom_stat_is_declared_and_seeded() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity_stat("morale")
        define_entity("hero", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10, morale = 7 },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let morale = registry
        .entity_stat("morale")
        .expect("morale is registered");
    assert_eq!(
        registry.entity("hero").unwrap().base_stat(morale),
        Some(FixedU64::from_num(7)),
    );
}

#[test]
fn custom_player_stat_is_declared() {
    let source = r#"
        define_player_stat("morale")
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert!(registry.has_player_stat("morale"));
    assert!(registry.player_stat("morale").is_some());
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
            targets = GROUND,
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("knight")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(100)
        .with_attack(12, 1, 1, 4, 2)
        .with_armor(4)
        .with_targets(LayerId::new(1))
        .with_bonus_damage_vs([("armored", 8u32), ("dragon", 15u32)])
        .with_energy(50, FixedU64::from_str("0.5").unwrap());

    assert_eq!(registry.entity("knight"), Some(&expected));
}

#[test]
fn parses_repairer_and_repair_ratio() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("building")

        define_entity("depot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 100 },
            cost = { gold = 200 },
            build_time = 20,
            repair_ratio = "0.5",
            tags = { "building" },
        })

        define_entity("worker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 20, repair_speed = "1.0", repair_cost_factor = "0.25",
                repair_range = 1,
            },
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = "present_stacking",
                cost = { mode = "pro_rata" },
                patience = 200,
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let depot = registry.entity("depot").expect("depot defined");
    assert_eq!(depot.repair_ratio, Some(FixedU64::from_str("0.5").unwrap()));

    let worker = registry.entity("worker").expect("worker defined");
    let repairer = worker.repairer.as_ref().expect("worker can repair");
    assert_eq!(repairer.repairs().collect::<Vec<_>>(), ["building"]);
    assert_eq!(repairer.presence(), WorkPresence::PresentStacking);
    assert_eq!(repairer.cost(), &RepairCost::ProRata);
    assert_eq!(repairer.patience(), Some(200));
    assert!(
        !repairer.self_repair(),
        "self-repair is off unless declared"
    );
}

#[test]
fn parses_transporter() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("footman", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 60, speed = "0.3", radius = "0.5", cargo_size = 1 },
            tags = { "infantry" },
        })

        define_entity("wagon", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 150, speed = "0.25", radius = "0.5", cargo_capacity = 6,
                load_range = 2, unload_range = 3, load_period = 4, unload_period = 8,
            },
            transporter = {
                carries = { "infantry", "footman" },
                boarding = "allies",
                fate = "eject",
                conduct = "fight",
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let wagon = registry.entity("wagon").expect("wagon defined");
    let transporter = wagon.transporter.as_ref().expect("wagon can transport");
    assert_eq!(
        transporter.carries().collect::<Vec<_>>(),
        ["footman", "infantry"]
    );
    assert_eq!(transporter.boarding(), BoardingPolicy::Allies);
    assert_eq!(transporter.passenger_fate(), PassengerFate::Eject);
    assert_eq!(transporter.conduct(), PassengerConduct::Fight);
    assert_eq!(
        wagon.base_stat(EntityStatId::CARGO_CAPACITY),
        Some(FixedU64::from_num(6))
    );
    assert_eq!(
        wagon.base_stat(EntityStatId::LOAD_RANGE),
        Some(FixedU64::from_num(2))
    );
    assert_eq!(
        wagon.base_stat(EntityStatId::UNLOAD_PERIOD),
        Some(FixedU64::from_num(8))
    );

    let footman = registry.entity("footman").expect("footman defined");
    assert_eq!(
        footman.base_stat(EntityStatId::CARGO_SIZE),
        Some(FixedU64::ONE)
    );
}

#[test]
fn parses_sheltering_transporter() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("cart", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100, cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            transporter = { carries = { "infantry" }, boarding = "own", fate = "destroy", conduct = "shelter" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let transporter = registry
        .entity("cart")
        .expect("cart defined")
        .transporter
        .as_ref()
        .expect("cart can transport");
    assert_eq!(transporter.boarding(), BoardingPolicy::Own);
    assert_eq!(transporter.passenger_fate(), PassengerFate::Destroy);
    assert_eq!(transporter.conduct(), PassengerConduct::Shelter);
}

#[test]
fn unknown_passenger_conduct_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("cart", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100, cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            transporter = { carries = { "infantry" }, boarding = "own", fate = "destroy", conduct = "mutiny" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown passenger conduct");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown passenger conduct 'mutiny'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_boarding_policy_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("cart", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100, cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            transporter = { carries = { "infantry" }, boarding = "anyone", fate = "destroy", conduct = "shelter" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown boarding policy");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown boarding policy 'anyone'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_passenger_fate_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("cart", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100, cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            transporter = { carries = { "infantry" }, boarding = "own", fate = "scatter", conduct = "shelter" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown passenger fate");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown passenger fate 'scatter'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn parses_flat_per_tick_repair_cost() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("building")

        define_entity("hauler", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, repair_speed = "1.0", repair_range = 1 },
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = "present",
                self_repair = true,
                cost = { mode = "per_tick", resources = { gold = 2 } },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let repairer = registry
        .entity("hauler")
        .expect("hauler defined")
        .repairer
        .as_ref()
        .expect("hauler can repair");

    assert_eq!(
        repairer.cost(),
        &RepairCost::PerTick(costs::cost([("gold", 2u32)]))
    );
    assert!(repairer.self_repair());
    assert_eq!(
        repairer.patience(),
        None,
        "an omitted patience waits indefinitely"
    );
}

#[test]
fn parses_medic_paying_energy_at_flat_rate() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("biological")

        define_entity("medic", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 45, max_energy = 200, energy_regen = "0.2",
                repair_speed = "1.0", repair_range = 2,
            },
            repairer = {
                repairs = { "biological" },
                rate = { mode = "per_tick", health = "1.0" },
                presence = "present",
                cost = { mode = "energy", per_health = "0.5" },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let medic = registry.entity("medic").expect("medic defined");
    let repairer = medic.repairer.as_ref().expect("medic can repair");

    assert_eq!(
        repairer.rate(),
        RepairRate::PerTick(FixedU64::from_str("1.0").unwrap())
    );
    assert_eq!(
        repairer.cost(),
        &RepairCost::Energy(FixedU64::from_str("0.5").unwrap())
    );
    assert_eq!(
        medic.base_stat(EntityStatId::REPAIR_RANGE),
        Some(FixedU64::from_num(2))
    );
    assert_eq!(
        repairer.presence(),
        WorkPresence::Present,
        "one medic to a patient"
    );
}

#[test]
fn parses_skill_with_buff_effect() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity_buff("haste", {
            duration = 20,
            stack = "refresh",
            modifiers = {
                { entity_stat = "damage", op = "percent", value = "1.0" },
            },
        })

        define_skill("battle_focus", {
            caster = "entity",
            cooldown = 5,
            cost = { energy = "30" },
            target = "caster",
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
    let haste = registry.entity_buff("haste").expect("haste buff defined");
    let battle_focus = registry.skill("battle_focus").expect("skill defined");

    // The entity references the skill by id, and the registered definition carries
    // the parsed caster, cooldown, cost, and effect.
    assert_eq!(mage.skills, vec![battle_focus]);
    assert_eq!(
        registry.skill_def(battle_focus),
        Some(&SkillDef {
            cooldown: 5,
            caster: SkillCaster::Entity {
                costs: vec![EntityCastCost::Energy(FixedU64::from_num(30))],
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::ApplyBuff(haste),
            },
            requires: Vec::new(),
        })
    );
}

#[test]
fn parses_skill_requirements() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity_buff("haste", {
            duration = 20,
            stack = "refresh",
            modifiers = {
                { entity_stat = "damage", op = "percent", value = "1.0" },
            },
        })
        define_research("arcana", { time = 100 })
        define_skill("war_secret", {
            caster = "entity",
            cooldown = 5,
            target = "caster",
            effect = { apply_buff = "haste" },
            requires = { "arcana" },
        })
        define_entity("mage", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 40 },
            skills = { "war_secret" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let war_secret = registry.skill("war_secret").expect("skill defined");
    assert_eq!(
        registry.skill_def(war_secret).unwrap().requires,
        vec!["arcana".to_string()]
    );
}

#[test]
#[should_panic(
    expected = "skill 'war_secret' requires 'arcana', which is not a registered entity type, tag, or research"
)]
fn undeclared_skill_requirement_panics_on_load() {
    let source = r#"
        define_player_buff("haste", {
            stack = "refresh",
            entity_modifiers = {
                { entity_stat = "speed", op = "percent", value = "0.5" },
            },
        })
        define_skill("war_secret", {
            caster = "player",
            cooldown = 5,
            effect = { remove_buff = "haste" },
            requires = { "arcana" },
        })
    "#;
    let _ = content::load(&engine(), source);
}

#[test]
fn parses_player_cast_skill() {
    let source = r#"
        define_resource("gold")

        define_player_buff("war_cry_haste", {
            duration = 10,
            stack = "refresh",
            entity_modifiers = {
                { entity_stat = "speed", op = "percent", value = "0.5" },
            },
        })

        define_skill("war_cry", {
            caster = "player",
            cooldown = 30,
            cost = { resources = { gold = 25 } },
            effect = { apply_buff = "war_cry_haste" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let war_cry = registry.skill("war_cry").expect("skill defined");
    let haste = registry.player_buff("war_cry_haste").expect("buff defined");
    assert_eq!(
        registry.skill_def(war_cry),
        Some(&SkillDef {
            cooldown: 30,
            caster: SkillCaster::Player {
                cost: costs::cost([("gold", 25)]),
                effect: PlayerCastEffect::ApplyBuff(haste),
            },
            requires: Vec::new(),
        })
    );
}

#[test]
fn unknown_skill_caster_errors() {
    let source = r#"
        define_skill("war_cry", {
            caster = "building",
            cooldown = 30,
            target = "caster_player",
            effect = { damage = "5" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown caster kind");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown skill caster 'building' (expected entity or player)")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_skill_target_errors() {
    let source = r#"
        define_skill("war_cry", {
            caster = "entity",
            cooldown = 30,
            target = "everyone",
            effect = { damage = "5" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown target");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown skill target 'everyone'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn player_cast_skill_with_target_errors() {
    let source = r#"
        define_skill("war_cry", {
            caster = "player",
            cooldown = 30,
            target = "caster_player",
            effect = { remove_buff = "haste" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a target on a player cast");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a player-cast skill takes no target")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn parses_player_buff_with_both_modifier_lists() {
    let source = r#"
        define_player_buff("prosperity", {
            duration = 10,
            stack = "refresh",
            player_modifiers = {
                { player_stat = "max_supply", op = "flat", value = "5" },
            },
            entity_modifiers = {
                { entity_stat = "speed", op = "percent", value = "0.5" },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let prosperity = registry.player_buff("prosperity").expect("buff defined");
    let def = registry.player_buff_def(prosperity);
    assert_eq!(
        def.player_modifiers,
        vec![PlayerModifier {
            stat: PlayerStatId::MAX_SUPPLY,
            op: ModifierOp::FlatAdd,
            magnitude: FixedI64::from_num(5),
        }]
    );
    assert_eq!(
        def.entity_modifiers,
        vec![EntityModifier {
            stat: EntityStatId::SPEED,
            op: ModifierOp::PercentAdd,
            magnitude: FixedI64::from_num(0.5),
        }]
    );
}

#[test]
fn player_stat_in_entity_modifier_list_errors() {
    let source = r#"
        define_entity_buff("confused", {
            duration = 10,
            stack = "refresh",
            modifiers = {
                { player_stat = "max_supply", op = "flat", value = "1" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a player stat in an entity modifier list");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("expected entity_stat, found player_stat")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn entity_stat_in_player_modifier_list_errors() {
    let source = r#"
        define_player_buff("confused", {
            duration = 10,
            stack = "refresh",
            player_modifiers = {
                { entity_stat = "speed", op = "flat", value = "1" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entity stat in a player modifier list");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("expected player_stat, found entity_stat")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn player_buff_without_modifier_lists_errors() {
    let source = r#"
        define_player_buff("aimless", {
            duration = 10,
            stack = "refresh",
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a player buff granting nothing");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("player buff must declare player_modifiers or entity_modifiers")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn skill_with_unknown_buff_errors() {
    let source = r#"
        define_skill("war_cry", {
            caster = "player",
            cooldown = 30,
            effect = { apply_buff = "war_cry_haste" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unregistered buff");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("player buff 'war_cry_haste' is not defined")),
        "unexpected error: {error:?}"
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
            targets = GROUND,
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("mortar")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(30)
        .with_attack(12, 6, 6, 10, 4)
        .with_targets(LayerId::new(1))
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
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
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
    assert_eq!(marine.base_stat(EntityStatId::SIGHT_RANGE), None);
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
// ─── Research ─────────────────────────────────────────────────────────────────
//

#[test]
fn parses_research_with_buff_and_requirements() {
    let source = r#"
        local ground = define_layer("ground")
        define_resource("gold")
        define_player_buff("sharp_blades", {
            stack = "ignore",
            entity_modifiers = {
                { entity_stat = "damage", op = "flat", value = "5" },
            },
        })
        define_research("smithing", {
            cost = { gold = 30 },
            time = 200,
            buff = "sharp_blades",
            requires = { "lab" },
        })
        define_research("tactics", {
            time = 100,
        })
        define_entity("lab", {
            location = { occupation = ground, size = { 2, 2 }, solidity = "solid" },
            researcher = { "smithing", "tactics" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let smithing = registry.research("smithing").expect("smithing registered");
    let expected = ResearchDef::new(
        costs::cost([("gold", 30)]),
        200,
        registry.player_buff("sharp_blades"),
        ["lab"],
    );
    assert_eq!(registry.research_def(smithing), Some(&expected));

    // An omitted cost is free, an omitted buff a pure unlock.
    let tactics = registry.research("tactics").expect("tactics registered");
    let expected = ResearchDef::new(costs::Cost::new(), 100, None, Vec::<String>::new());
    assert_eq!(registry.research_def(tactics), Some(&expected));

    let lab = registry.entity("lab").expect("lab registered");
    let researcher = lab.researcher.as_ref().expect("lab hosts researches");
    assert!(researcher.can_research(smithing));
    assert!(researcher.can_research(tactics));
}

#[test]
fn loads_declared_requirements_onto_entities() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("blacksmith", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("mortar", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { "blacksmith" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert_eq!(
        registry.entity("mortar").unwrap().requires,
        vec!["blacksmith".to_string()]
    );
}

#[test]
fn research_with_unknown_buff_errors() {
    let source = r#"
        define_research("smithing", {
            time = 200,
            buff = "sharp_blades",
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown buff name");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("player buff 'sharp_blades' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn researcher_with_unknown_research_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("lab", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            researcher = { "smithing" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown research name");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("research 'smithing' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
#[should_panic(
    expected = "entity type 'mortar' requires 'blacksmith', which is not a registered entity type, tag, or research"
)]
fn undeclared_requirement_panics_on_load() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("mortar", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { "blacksmith" },
        })
    "#;
    let _ = content::load(&engine(), source);
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
        CellSize::ONE,
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

    let expected = EntityTypeDef::new("gryphon").with_location(
        LayerId::new(2),
        CellSize::ONE,
        Solidity::Solid,
    );
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
fn repairer_without_rate_errors() {
    // No default: mending a structure against its build time and patching up a
    // casualty at a flat rate are both ordinary, so content states which it is.
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("building")

        define_entity("worker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, repair_speed = "1.0", repair_range = 1 },
            repairer = { repairs = { "building" }, presence = "present" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a repairer must state its rate");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("field 'rate'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn repairer_without_cost_errors() {
    // Free work is a balance stance, not an absence — `{ mode = "free" }` says so,
    // and requiring it keeps a misspelled field from quietly meaning free.
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("building")

        define_entity("worker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, repair_speed = "1.0", repair_range = 1 },
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = "present",
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a repairer must state what its work costs");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("field 'cost'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_repair_rate_mode_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("biological")

        define_entity("medic", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, repair_speed = "1.0", repair_range = 1 },
            repairer = {
                repairs = { "biological" },
                rate = { mode = "instant" },
                presence = "present",
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown repair rate mode");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown repair rate mode 'instant'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_work_presence_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("building")

        define_entity("worker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, repair_speed = "1.0" },
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = "lurking",
                cost = { mode = "free" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown work presence");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("unknown work presence 'lurking'")),
        "unexpected error: {error:?}"
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
            speed = "0.3", radius = "0.5", max_health = 40,
            damage = 6, attack_range = 4, attack_period = 7, damage_point = 3,
        },
        dying = { time = 2 },
        targets = GROUND,
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
        stats = { speed = "0.3", radius = "0.5", max_health = 30, build_range = 1, harvest_range = 1 },
        dying = { time = 2 },
        cost = { gold = 50 },
        train_time = 40,
        builder = { builds = { "town_hall" }, presence = "hidden" },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = "hidden" },
            wood = { capacity = 5, time = 20, presence = "present" },
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

#[test]
fn morph_transitions_round_trip() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("winged")
        define_entity_stat("morph_time")

        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5", max_energy = 50, morph_time = 20 },
            morphs = {
                { into = "flier",
                  time = { stat = "morph_time" },
                  placement = "revalidate",
                  cancel = "committed",
                  cost = { energy = "20" },
                  requires = { "winged" } },
                { into = "statue",
                  time = 40,
                  placement = "reserve",
                  cancel = "refundable",
                  cost = { resources = { gold = 30 } } },
            },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5" },
        })
        define_entity("statue", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 100 },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    let walker = registry.entity("walker").expect("walker is registered");
    let [first, second] = walker.morphs.as_slice() else {
        panic!("walker declares exactly two transitions");
    };

    let morph_time = registry
        .entity_stat("morph_time")
        .expect("morph_time is declared");
    assert_eq!(first.into_type(), "flier");
    assert_eq!(first.time(), MorphTime::Stat(morph_time));
    assert_eq!(first.placement(), MorphPlacement::Revalidate);
    assert_eq!(first.cancel(), MorphCancel::Committed);
    assert_eq!(
        first.costs(),
        [EntityCastCost::Energy(FixedU64::from_num(20))]
    );
    assert_eq!(first.requires(), ["winged"]);

    assert_eq!(second.into_type(), "statue");
    assert_eq!(second.time(), MorphTime::Constant(40));
    assert_eq!(second.placement(), MorphPlacement::Reserve);
    assert_eq!(second.cancel(), MorphCancel::Refundable);
    assert_eq!(
        second.costs(),
        [EntityCastCost::Resources(costs::cost([("gold", 30)]))]
    );
    assert!(second.requires().is_empty());
}

#[test]
fn unknown_morph_placement_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "hover", cancel = "committed" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown morph placement");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph placement must be 'reserve' or 'revalidate', found 'hover'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_morph_cancel_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "reserve", cancel = "maybe" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown morph cancel");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph cancel must be 'committed', 'forfeit', or 'refundable', found 'maybe'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn morph_time_of_wrong_shape_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5" },
            morphs = {
                { into = "flier", time = "fast", placement = "reserve", cancel = "committed" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a morph time that is neither ticks nor a stat table");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph time must be a tick count or a { stat = ... } table, found string")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_morph_time_stat_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, radius = "0.5" },
            morphs = {
                { into = "flier", time = { stat = "bogus" }, placement = "reserve", cancel = "committed" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a morph time naming an unknown stat");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph time stat 'bogus' is not defined")),
        "unexpected error: {error:?}"
    );
}
