//! Loading content into a `ContentRegistry`: the definitions round-trip to
//! the same `EntityTypeDef`s the Rust builder produces, and malformed scripts
//! surface as errors rather than panics. The contract holds for any engine;
//! [`engine`] picks the binding the suite runs against.

use ferrets_content::{
    annex::{AloneConduct, AnnexClaim, AnnexLife, AnnexWork},
    attack::{AttackDef, Delivery, Weapon},
    brood::{BroodlingDef, Lingering, OrphanFate},
    build::BuilderAttendance,
    costs,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        FieldAction, FieldAffiliation, FieldCoverage, FieldDecay, FieldEffect, FieldEffectKind,
        FieldGrowth, FieldPlacement, FieldSide, FieldSourceDef, FieldVision,
    },
    location::Solidity,
    morph::{MorphCancel, MorphInterrupted, MorphPlacement, MorphReason},
    period::Period,
    player_stats::PlayerStatId,
    repair::{RepairCost, RepairRate},
    requirement::Requirement,
    research::ResearchDef,
    resource::{Banking, Sources},
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillDef,
    },
    splash::{SplashDef, SplashShape},
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp, PlayerModifier},
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::LayerId};
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
            FixedU64::from_str("2").unwrap(),
            FixedU64::from_num(30),
            FixedU64::from_num(30),
        )
        .with_health(40)
        .with_dying(2, None)
        .with_attack(
            AttackDef::new(Weapon::new(LayerId::new(1), Delivery::Instant, None)),
            6,
            4,
            4,
            7,
            3,
        )
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
            attack = { targets = GROUND },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("scout")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(20)
        .with_attack(
            AttackDef::new(Weapon::new(LayerId::new(1), Delivery::Instant, None)),
            2,
            3,
            7,
            4,
            2,
        );

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

        define_tag("armored")
        define_tag("dragon")

        define_entity("knight", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100,
                damage = 12, attack_range = 1, attack_period = 4, damage_point = 2,
                armor = 4, max_energy = 50, energy_regen = "0.5",
            },
            bonus_damage_vs = { armored = 8, dragon = 15 },
            attack = { targets = GROUND },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("knight")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(100)
        .with_armor(4)
        .with_attack(
            AttackDef::new(Weapon::new(LayerId::new(1), Delivery::Instant, None)),
            12,
            1,
            1,
            4,
            2,
        )
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
                presence = { present = { crew = "any" } },
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
    assert_eq!(
        *repairer.presence(),
        WorkPresence::Present {
            crew: CrewLimit::Unlimited,
        }
    );
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
            stats = { max_health = 60, speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", cargo_size = 1 },
            tags = { "infantry" },
        })

        define_entity("wagon", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 150, speed = "0.25", turn_rate = 30, pivot_rate = 30, radius = "0.5", cargo_capacity = 6,
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("passenger conduct must be 'shelter' or 'fight', found 'mutiny'")),
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("boarding policy must be 'own' or 'allies', found 'anyone'")),
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("passenger fate must be 'destroy' or 'eject', found 'scatter'")),
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
                presence = { present = { crew = 1 } },
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
                presence = { present = { crew = 1 } },
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
        *repairer.presence(),
        WorkPresence::Present {
            crew: CrewLimit::ONE
        },
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
            requires = { { research = "arcana" } },
        })
        define_entity("mage", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 40 },
            skills = { "war_secret" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let war_secret = registry.skill("war_secret").expect("skill defined");
    let arcana = registry.research("arcana").expect("research defined");
    assert_eq!(
        registry.skill_def(war_secret).unwrap().requires,
        vec![Requirement::Research(arcana)]
    );
}

#[test]
fn requirement_naming_undeclared_research_is_rejected() {
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
            requires = { { research = "arcana" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a research nothing declared");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("research 'arcana' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_naming_no_kind_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10 },
            requires = { { forge = "blacksmith" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry naming no kind");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains(
            "a requirement must name exactly one of entity_type, tag, research, or annexed"
        )),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_naming_two_kinds_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("workshop")
        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10 },
            requires = { { entity_type = "blacksmith", tag = "workshop" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry naming two kinds");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains(
            "a requirement must name exactly one of entity_type, tag, research, or annexed"
        )),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_naming_two_kinds_reports_shape_before_lookup() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10 },
            requires = { { entity_type = "blacksmith", research = "arcana" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry naming two kinds");
    };
    // The shape is judged first, so the entry reads as the shape error it is
    // and not as the failed lookup of a research it should never have asked for.
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains(
            "a requirement must name exactly one of entity_type, tag, research, or annexed"
        )),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_that_is_bare_name_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 10 },
            requires = { "blacksmith" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a bare name");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a requirement must be")),
        "unexpected error: {error:?}"
    );
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("skill caster must be 'entity' or 'player', found 'building'")),
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("skill target must be 'caster', 'ally', 'enemy', or 'position', found 'everyone'")),
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
            attack = {
                targets = GROUND,
                projectile = "shell",
                splash = {
                    shape = "circular",
                    bands = { {1, "0.5"}, {2, "0.25"} },
                    layers = GROUND,
                    friendly_fire = true,
                },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("mortar")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_health(30)
        .with_attack(
            AttackDef::new(Weapon::new(
                LayerId::new(1),
                Delivery::Projectile(registry.projectile("shell").expect("shell is registered")),
                Some(SplashDef::new(
                    SplashShape::Circular,
                    vec![
                        (1, FixedU64::from_str("0.5").unwrap()),
                        (2, FixedU64::from_str("0.25").unwrap()),
                    ],
                    LayerId::new(1),
                    true,
                )),
            )),
            12,
            6,
            6,
            10,
            4,
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
fn weapon_without_targets_is_rejected() {
    // The layers a weapon reaches are the one thing it cannot leave out: a weapon
    // that reaches nothing could never fire, so the rest says nothing without it.
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { damage = 12, attack_range = 6, attack_period = 10, damage_point = 4 },
            attack = {},
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a weapon reaching nothing must be rejected");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("field 'targets'")),
        "the error must name the missing 'targets' field, got: {error}"
    );
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
        "content error: attack aim must be 'entity' or 'position', found 'sideways'"
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
            requires = { { entity_type = "lab" } },
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
        [Requirement::EntityType("lab".to_string())],
    );
    assert_eq!(registry.research_def(smithing), Some(&expected));

    // An omitted cost is free, an omitted buff a pure unlock.
    let tactics = registry.research("tactics").expect("tactics registered");
    let expected = ResearchDef::new(costs::Cost::new(), 100, None, Vec::new());
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
            requires = { { entity_type = "blacksmith" } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    assert_eq!(
        registry.entity("mortar").unwrap().requires,
        vec![Requirement::EntityType("blacksmith".to_string())]
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
    expected = "entity type 'mortar' requires the entity type 'blacksmith', which is not registered"
)]
fn undeclared_requirement_panics_on_load() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("mortar", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { entity_type = "blacksmith" } },
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
            repairer = { repairs = { "building" }, presence = { present = { crew = 1 } } },
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
                presence = { present = { crew = 1 } },
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
                presence = { present = { crew = 1 } },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown repair rate mode");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("repair rate mode must be 'production' or 'per_tick', found 'instant'")),
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("work presence must be a { hidden = ... } table, a { present = ... } table, or an { attached = ... } table, found 'lurking'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn docks_and_annex_read_their_terms() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("building")

        define_entity("barracks", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = { max_health = 100, build_range = 1 },
            tags = { "building" },
            builder = { builds = { "tech_lab" }, attendance = { present = { crew = 1 } } },
            docks = { { at = { 2, 0 }, accepts = { "tech_lab" } } },
        })
        define_entity("tech_lab", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 40, health_drain = "2" },
            tags = { "building" },
            cost = { gold = 25 },
            build_time = 10,
            annex = { alone = { work = "idles", life = { fades = "2" } }, claim = "seized" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    let barracks = registry.entity("barracks").expect("barracks defined");
    let dock = barracks.docks.first().expect("it offers one dock");
    assert_eq!(dock.at(), CellPos::new(2, 0));
    assert!(dock.accepts("tech_lab") && !dock.accepts("barracks"));

    let annex = registry
        .entity("tech_lab")
        .expect("tech_lab defined")
        .annex
        .expect("it is an annex");
    assert_eq!(
        annex.alone(),
        AloneConduct::Standing {
            work: AnnexWork::Idles,
            life: AnnexLife::Fades {
                per_tick: FixedU64::lit("2")
            },
        }
    );
    assert_eq!(annex.claim(), AnnexClaim::Seized);
}

#[test]
fn watch_effect_reads_its_radius_and_duration() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_skill("scan", {
            cooldown = 10,
            caster = "entity",
            target = "position",
            effect = { watch = { radius = 6, duration = 40 } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let skill = registry
        .skill("scan")
        .and_then(|id| registry.skill_def(id))
        .expect("scan defined");
    let SkillCaster::Entity { effect, .. } = &skill.caster else {
        panic!("scan is cast by an entity");
    };
    assert_eq!(
        *effect,
        EntityCastEffect::Watch {
            radius: 6,
            duration: 40
        }
    );
}

#[test]
fn presence_table_reads_crew_limit_and_harvest_reads_its_sources() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")

        define_entity("seam", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            resource_source = { kind = "gold", depletion = "destroy" },
        })
        define_entity("refinery", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 100 },
            resource_source = { kind = "gold", depletion = "persist" },
        })
        define_entity("tapper", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1 },
            resource_carrier = {
                gold = {
                    capacity = 5, time = 20,
                    presence = { hidden = { crew = 3 } },
                    sources = { "refinery" },
                },
            },
        })
        define_entity("gang", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1 },
            resource_carrier = {
                gold = { capacity = 5, time = 20, presence = { present = { crew = "any" } } },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    // Three at a time, off the map while they work, and only out of a refinery.
    let gold = registry
        .entity("tapper")
        .unwrap()
        .resource_carrier
        .as_ref()
        .unwrap()
        .harvest_data("gold")
        .unwrap()
        .clone();
    assert_eq!(
        gold.presence(),
        &WorkPresence::Hidden {
            crew: CrewLimit::limit(3)
        }
    );
    assert_eq!(gold.sources(), &Sources::only(["refinery"]));
    assert!(!gold.admits_source("seam"));

    // A crew of "any", and no source named, which takes every gold source.
    let gang = registry
        .entity("gang")
        .unwrap()
        .resource_carrier
        .as_ref()
        .unwrap()
        .harvest_data("gold")
        .unwrap()
        .clone();
    assert_eq!(
        gang.presence(),
        &WorkPresence::Present {
            crew: CrewLimit::Unlimited
        }
    );
    assert!(gang.admits_source("seam") && gang.admits_source("refinery"));
}

#[test]
#[should_panic(expected = "a crew limit must admit at least one worker")]
fn crew_limit_of_zero_panics() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")

        define_entity("seam", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            resource_source = { kind = "gold", depletion = "destroy" },
        })
        define_entity("tapper", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1 },
            resource_carrier = {
                gold = { capacity = 5, time = 20, presence = { hidden = { crew = 0 } } },
            },
        })
    "#;
    // A crew of nobody is a value invariant, so it is the constructor that
    // refuses it rather than the reader.
    let _ = content::load(&engine(), source);
}

#[test]
fn attached_presence_reads_berths_and_stance() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("wood")
        define_resource("gold")
        define_tag("building")

        define_entity("tree", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            resource_source = { kind = "wood", depletion = "destroy" },
            berths = { canopy = { points = { { "0.5", "0.5" } } } },
        })
        define_entity("lodge", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = { max_health = 100 },
            cost = { gold = 10 },
            build_time = 4,
            berths = {
                rim = { points = { { 1, 0 }, { "1.8", "1.0" }, { 1, "1.8" }, { "0.2", 1 } }, slots = 2 },
                ledge = { points = { { 0, 0 }, { 1, 1 }, { 0, 1 } } },
            },
        })
        define_entity("sprite", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1, build_range = 1 },
            resource_carrier = {
                wood = {
                    capacity = 5, time = 20, drain = 0, banking = "direct",
                    presence = { attached = { berths = "canopy", stance = "still" } },
                },
            },
            builder = {
                builds = { "lodge" },
                attendance = { attached = { berths = "rim", stance = { roaming = { speed = "0.05", dwell = 10 } } } },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    let tree = registry.entity("tree").unwrap();
    let canopy = tree.berths.as_ref().unwrap().group("canopy").unwrap();
    assert_eq!(
        canopy.points(),
        &[FixedVec2::new(FixedI64::lit("0.5"), FixedI64::lit("0.5"))]
    );
    assert_eq!(canopy.slots(), 1);
    let lodge = registry.entity("lodge").unwrap();
    let rim = lodge.berths.as_ref().unwrap().group("rim").unwrap();
    assert_eq!(rim.points().len(), 4);
    assert_eq!(rim.slots(), 2);
    let ledge = lodge.berths.as_ref().unwrap().group("ledge").unwrap();
    assert_eq!(ledge.points().len(), 3);
    assert_eq!(
        ledge.slots(),
        3,
        "one worker per point unless said otherwise"
    );
    // Whole numbers and decimal strings both read as fixed-point.
    assert_eq!(
        rim.points()[0],
        FixedVec2::new(FixedI64::ONE, FixedI64::ZERO)
    );
    assert_eq!(
        rim.points()[1],
        FixedVec2::new(FixedI64::lit("1.8"), FixedI64::ONE)
    );

    let sprite = registry.entity("sprite").unwrap();
    let wood = sprite
        .resource_carrier
        .as_ref()
        .unwrap()
        .harvest_data("wood")
        .unwrap();
    assert_eq!(
        *wood.presence(),
        WorkPresence::Attached(Attachment::new("canopy", BerthStance::Still))
    );
    assert_eq!(wood.drain(), 0);
    assert_eq!(wood.banking(), Banking::Direct);
    assert_eq!(
        *sprite.builder.as_ref().unwrap().attendance(),
        BuilderAttendance::Crew(WorkPresence::Attached(Attachment::new(
            "rim",
            BerthStance::Roaming {
                speed: FixedU64::lit("0.05"),
                dwell: 10,
            },
        )))
    );
}

#[test]
fn overbuilding_type_reads_its_source() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("building")

        define_entity("mine", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            resource_source = { kind = "gold", depletion = "destroy" },
        })
        define_entity("shaft_house", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = { max_health = 100 },
            cost = { gold = 10 },
            build_time = 4,
            resource_source = { kind = "gold", depletion = "destroy" },
            overbuilds = "mine",
            tags = { "building" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    assert_eq!(
        registry
            .entity("shaft_house")
            .unwrap()
            .overbuilds
            .as_deref(),
        Some("mine")
    );
}

#[test]
fn orbit_stance_reads_radius_and_period() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("wood")

        define_entity("tree", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            resource_source = { kind = "wood", depletion = "destroy" },
            berths = { canopy = { points = { { "0.5", "0.5" } } } },
        })
        define_entity("sprite", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1 },
            resource_carrier = {
                wood = {
                    capacity = 5, time = 20,
                    presence = { attached = { berths = "canopy", stance = { orbit = { radius = "0.3", period = 40 } } } },
                },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let sprite = registry.entity("sprite").unwrap();
    let wood = sprite
        .resource_carrier
        .as_ref()
        .unwrap()
        .harvest_data("wood")
        .unwrap();
    assert_eq!(
        *wood.presence(),
        WorkPresence::Attached(Attachment::new(
            "canopy",
            BerthStance::Orbit {
                radius: FixedU64::lit("0.3"),
                period: 40,
            },
        ))
    );
}

#[test]
fn circling_stance_reads_speed_and_dwell() {
    let stance = "{ circling = { speed = \"0.25\", dwell = 8 } }";
    let source = carrier_content("{ { \"0.5\", \"0.5\" }, { \"0.5\", \"0.9\" } }", stance);
    let registry = content::load(&engine(), &source).expect("content loads");
    let sprite = registry.entity("sprite").unwrap();
    let wood = sprite
        .resource_carrier
        .as_ref()
        .unwrap()
        .harvest_data("wood")
        .unwrap();
    assert_eq!(
        *wood.presence(),
        WorkPresence::Attached(Attachment::new(
            "canopy",
            BerthStance::Circling {
                speed: FixedU64::lit("0.25"),
                dwell: 8,
            },
        ))
    );
}

#[test]
fn berth_point_of_float_errors() {
    let Err(error) = content::load(&engine(), &carrier_content("{ { 0.5, 0.5 } }", "\"still\""))
    else {
        panic!("must reject a float berth point");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("berth x must be an integer or a decimal string, got number")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn berth_point_that_is_not_a_pair_errors() {
    let Err(error) = content::load(&engine(), &carrier_content("{ { 1, 1, 1 } }", "\"still\""))
    else {
        panic!("must reject a berth point that is not a pair");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("berth group 'canopy' points must be {x, y} pairs")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn stance_speed_of_float_errors() {
    let stance = "{ roaming = { speed = 0.25, dwell = 4 } }";
    let Err(error) = content::load(
        &engine(),
        &carrier_content("{ { 0, 0 }, { 1, 1 } }", stance),
    ) else {
        panic!("must reject a float berth-to-berth speed");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("berth-to-berth speed must be a non-negative integer or a decimal string, got number")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn stance_table_of_two_movements_errors() {
    let stance =
        "{ roaming = { speed = \"0.25\", dwell = 4 }, orbit = { radius = \"0.3\", period = 8 } }";
    let Err(error) = content::load(
        &engine(),
        &carrier_content("{ { 0, 0 }, { 1, 1 } }", stance),
    ) else {
        panic!("must reject a stance table naming two movements");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a berth stance table must have exactly one of 'circling', 'roaming' or 'orbit'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_berth_stance_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("wood")

        define_entity("tree", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            resource_source = { kind = "wood", depletion = "destroy" },
            berths = { canopy = { points = { { "0.5", "0.5" } } } },
        })
        define_entity("sprite", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20, harvest_range = 1 },
            resource_carrier = {
                wood = {
                    capacity = 5, time = 20,
                    presence = { attached = { berths = "canopy", stance = "wandering" } },
                },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown berth stance");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("berth stance must be 'still', a { circling = ... } table, a { roaming = ... } table, or an { orbit = ... } table, found 'wandering'")),
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
// ─── Fields ───────────────────────────────────────────────────────────────────
//

#[test]
fn parses_fields_sources_placement_and_effects() {
    let registry = content::load(&engine(), FIELDS).expect("load content");
    let creep = registry.field("creep").expect("creep defined");
    let power = registry.field("power").expect("power defined");
    let blight = registry.field("blight").expect("blight defined");

    assert_eq!(
        registry.field_def(creep).decay(),
        FieldDecay::Gradual { cycle: 9 }
    );
    assert_eq!(registry.field_def(power).decay(), FieldDecay::Instant);
    assert_eq!(registry.field_def(blight).decay(), FieldDecay::Never);
    assert_eq!(registry.field_def(creep).vision(), FieldVision::Watched);
    assert_eq!(registry.field_def(power).vision(), FieldVision::Dark);
    assert_eq!(
        registry.field_def(creep).layer(),
        LayerMask::from(LayerId::new(1))
    );

    assert_eq!(
        registry.entity("hive").unwrap().field_sources,
        vec![FieldSourceDef::new(
            creep,
            10,
            FieldGrowth::Gradual {
                cycle: 9,
                initial_radius: 1,
            },
            Some(1),
        )]
    );
    let pylon = registry.entity("pylon").unwrap();
    assert_eq!(
        pylon.field_sources,
        vec![FieldSourceDef::new(power, 6, FieldGrowth::Instant, None)]
    );
    assert_eq!(
        pylon.on_stand,
        vec![StandingAct::Field {
            field: creep,
            radius: 6,
            action: FieldAction::Clear,
        }]
    );
    let gateway = registry.entity("gateway").unwrap();
    assert_eq!(
        gateway.field_placement,
        vec![
            FieldPlacement::Requires {
                field: power,
                of: FieldAffiliation::Own,
                coverage: FieldCoverage::Anchor,
            },
            FieldPlacement::Forbids { field: creep },
        ]
    );
    assert_eq!(
        gateway.field_effects,
        vec![FieldEffect::new(
            power,
            FieldAffiliation::Own,
            FieldSide::Outside,
            FieldEffectKind::Disabled,
        )]
    );
    assert_eq!(
        registry.entity("zergling").unwrap().field_effects,
        vec![FieldEffect::new(
            creep,
            FieldAffiliation::Anyone,
            FieldSide::Inside,
            FieldEffectKind::Modifiers(vec![EntityModifier {
                stat: EntityStatId::SPEED,
                op: ModifierOp::PercentAdd,
                magnitude: FixedI64::from_str("0.3").unwrap(),
            }]),
        )]
    );
    let spew = registry.skill("spew").expect("skill defined");
    assert_eq!(
        registry.skill_def(spew),
        Some(&SkillDef {
            cooldown: 20,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Position,
                effect: EntityCastEffect::Field {
                    field: creep,
                    radius: 1,
                    action: FieldAction::Cover,
                },
            },
            requires: Vec::new(),
        })
    );
}

#[test]
fn unknown_field_name_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hive", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_sources = { { field = "creep", radius = 3, growth = "instant" } },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("undefined field");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("field 'creep' is not defined")),
        "{error:?}"
    );
}

#[test]
fn field_placement_rule_names_exactly_one_verb() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("creep", { layer = GROUND, decay = "never" })
        define_entity("spore", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_placement = { { requires = "creep", forbids = "creep", of = "anyone", coverage = "anchor" } },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("two verbs");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("exactly one of requires or forbids")),
        "{error:?}"
    );
}

#[test]
fn unknown_field_vision_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("creep", { layer = GROUND, decay = "never", vision = "glowing" })
    "#;
    let error = content::load(&engine(), source).err().expect("bad vision");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("field vision must be 'dark' or 'watched', found 'glowing'")),
        "{error:?}"
    );
}

#[test]
fn unknown_field_decay_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("creep", { layer = GROUND, decay = "slowly" })
    "#;
    let error = content::load(&engine(), source).err().expect("bad decay");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("field decay must be")),
        "{error:?}"
    );
}

//
// ─── Breeder, broodling, interruption and reason ─────────────────────────────
//

#[test]
fn breeder_and_broodling_round_trip() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity_stat("brood_period")
        define_entity("hatch", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 300, brood_period = 12 },
            berths = { brood = { points = { { "0.5", "3.5" }, { "-1", "1.0" } }, slots = 2 } },
            breeder = { breeds = "grub", period = { stat = "brood_period" }, limit = 2, initial = 1,
                        orphans = { linger = { reseat = { distance = 3 } } } },
        })
        define_entity("grub", {
            location = { occupation = GROUND, size = 1, solidity = "passable" },
            stats = { max_health = 25 },
            broodling = { berths = "brood", stance = "still" },
        })
        define_entity("pen", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = { max_health = 100 },
            berths = { sty = { points = { { "0.5", "2.5" }, { "1.5", "2.5" }, { "0.5", "-0.5" }, { "1.5", "-0.5" } }, slots = 4 } },
            breeder = { breeds = "piglet", period = 30, limit = 4, orphans = "perish" },
        })
        define_entity("piglet", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 20 },
            broodling = { berths = "sty", stance = { roaming = { speed = "0.05", dwell = 40 } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    let hatch = registry.entity("hatch").expect("hatch is registered");
    let brood = hatch.breeder.as_ref().expect("the hatch breeds");
    let brood_period = registry
        .entity_stat("brood_period")
        .expect("brood_period is declared");
    assert_eq!(brood.breeds(), "grub");
    assert_eq!(brood.period(), Period::Stat(brood_period));
    assert_eq!(brood.limit(), 2);
    assert_eq!(brood.initial(), 1);
    assert_eq!(
        brood.orphans(),
        OrphanFate::Linger(Lingering::Reseat { distance: 3 })
    );
    // Berth points read either sign.
    let group = hatch
        .berths
        .as_ref()
        .and_then(|berths| berths.group("brood"))
        .expect("the hatch offers the brood group");
    assert_eq!(
        group.points(),
        &[
            FixedVec2::new(FixedI64::lit("0.5"), FixedI64::lit("3.5")),
            FixedVec2::new(FixedI64::lit("-1"), FixedI64::ONE),
        ]
    );

    let grub = registry.entity("grub").expect("grub is registered");
    assert_eq!(
        grub.broodling.as_ref().map(BroodlingDef::attachment),
        Some(&Attachment::new("brood", BerthStance::Still))
    );

    let pen = registry.entity("pen").expect("pen is registered");
    let brood = pen.breeder.as_ref().expect("the pen breeds");
    assert_eq!(brood.period(), Period::Constant(30));
    assert_eq!(brood.initial(), 0);
    assert_eq!(brood.orphans(), OrphanFate::Perish);
    let piglet = registry.entity("piglet").expect("piglet is registered");
    assert_eq!(
        piglet.broodling.as_ref().map(BroodlingDef::attachment),
        Some(&Attachment::new(
            "sty",
            BerthStance::Roaming {
                speed: FixedU64::lit("0.05"),
                dwell: 40,
            }
        ))
    );
}

#[test]
fn unknown_orphan_fate_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hatch", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 300 },
            breeder = { breeds = "grub", period = 12, limit = 1, orphans = "wander" },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("bad fate");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("orphan fate must be 'perish', 'linger', or a { linger = { reseat = { distance = ... } } } table, found 'wander'")),
        "{error:?}"
    );
}

#[test]
fn unknown_morph_reason_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "reserve", cancel = "committed", reason = "growth" },
            },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("bad reason");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("morph reason must be 'production' or 'change', found 'growth'")),
        "{error:?}"
    );
}

#[test]
fn lingering_without_reseat_distance_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hatch", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 300 },
            breeder = { breeds = "grub", period = 12, limit = 1, orphans = { linger = { reseat = {} } } },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad lingering");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("distance")),
        "{error:?}"
    );
}

#[test]
fn brood_period_of_wrong_shape_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hatch", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 300 },
            breeder = { breeds = "grub", period = "soon", limit = 1, orphans = "perish" },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("bad period");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("brood period")),
        "{error:?}"
    );
}

#[test]
fn morph_interrupted_and_reason_read_and_default() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "nearby", cancel = "refundable",
                  interrupted = "dies", reason = "production" },
                { into = "statue", time = 20, placement = "revalidate", cancel = "committed" },
            },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
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
    assert_eq!(first.placement(), MorphPlacement::Nearby);
    assert_eq!(first.interrupted(), MorphInterrupted::Dies);
    assert_eq!(first.reason(), MorphReason::Production);
    assert_eq!(second.interrupted(), MorphInterrupted::Reverts);
    assert_eq!(second.reason(), MorphReason::Change);
}

#[test]
fn unknown_morph_interrupted_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "reserve", cancel = "committed", interrupted = "vanishes" },
            },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad interruption");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("morph interrupted must be 'reverts' or 'dies', found 'vanishes'")),
        "{error:?}"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// A tree with the given berth `points` and a carrier that sits in them with
/// the given `stance`, both written as Lua.
fn carrier_content(points: &str, stance: &str) -> String {
    format!(
        r#"
        local GROUND = define_layer("ground")
        define_resource("wood")

        define_entity("tree", {{
            location = {{ occupation = GROUND, size = 1, solidity = "solid" }},
            resource_source = {{ kind = "wood", depletion = "destroy" }},
            berths = {{ canopy = {{ points = {points} }} }},
        }})
        define_entity("sprite", {{
            location = {{ occupation = GROUND, size = 1, solidity = "solid" }},
            stats = {{ max_health = 20, harvest_range = 1 }},
            resource_carrier = {{
                wood = {{
                    capacity = 5, time = 20,
                    presence = {{ attached = {{ berths = "canopy", stance = {stance} }} }},
                }},
            }},
        }})
    "#
    )
}

/// One self-contained ranged unit (no production catalogue).
const ARCHER: &str = r#"
    local GROUND = define_layer("ground")

    define_race("human")

    define_resource("gold")

    define_entity("archer", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 40,
            damage = 6, attack_range = 4, attack_period = 7, damage_point = 3,
        },
        dying = { time = 2 },
        attack = { targets = GROUND },
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
        stats = { speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", max_health = 30, build_range = 1, harvest_range = 1 },
        dying = { time = 2 },
        cost = { gold = 50 },
        train_time = 40,
        builder = { builds = { "town_hall" }, attendance = { hidden = { crew = 1 } } },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } } },
            wood = { capacity = 5, time = 20, presence = { present = { crew = 1 } } },
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

const FIELDS: &str = r#"
    local GROUND = define_layer("ground")

    define_field("creep", { layer = GROUND, decay = { cycle = 9 }, vision = "watched" })
    define_field("power", { layer = GROUND, decay = "instant" })
    define_field("blight", { layer = GROUND, decay = "never" })

    define_entity("hive", {
        location = { occupation = GROUND, size = 2, solidity = "solid" },
        stats = { max_health = 100 },
        build_time = 20,
        field_sources = {
            { field = "creep", radius = 10, growth = { cycle = 9, initial_radius = 1 }, while_constructing = 1 },
        },
    })

    define_entity("pylon", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 100 },
        field_sources = {
            { field = "power", radius = 6, growth = "instant" },
        },
        on_stand = {
            { field = { field = "creep", radius = 6, action = "clear" } },
        },
    })

    define_entity("gateway", {
        location = { occupation = GROUND, size = 2, solidity = "solid" },
        stats = { max_health = 100 },
        field_placement = {
            { requires = "power", of = "own", coverage = "anchor" },
            { forbids = "creep" },
        },
        field_effects = {
            { field = "power", of = "own", outside = "disabled" },
        },
    })

    define_entity("zergling", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 20 },
        field_effects = {
            { field = "creep", of = "anyone", inside = {
                modifiers = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } },
        },
    })

    define_skill("spew", {
        caster = "entity",
        cooldown = 20,
        target = "position",
        effect = { field = { field = "creep", radius = 1, action = "cover" } },
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
            attack = {{ targets = GROUND, {block} }},
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
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5", max_energy = 50, morph_time = 20 },
            morphs = {
                { into = "flier",
                  time = { stat = "morph_time" },
                  placement = "revalidate",
                  cancel = "committed",
                  cost = { energy = "20" },
                  requires = { { tag = "winged" } } },
                { into = "statue",
                  via = "chrysalis",
                  time = 40,
                  placement = "reserve",
                  cancel = "refundable",
                  cost = { resources = { gold = 30 } } },
            },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
        })
        define_entity("chrysalis", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { max_health = 60 },
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
    assert_eq!(first.time(), Period::Stat(morph_time));
    assert_eq!(first.placement(), MorphPlacement::Revalidate);
    assert_eq!(first.cancel(), MorphCancel::Committed);
    assert_eq!(
        first.costs(),
        [EntityCastCost::Energy(FixedU64::from_num(20))]
    );
    assert_eq!(first.requires(), [Requirement::Tag("winged".to_string())]);

    assert_eq!(second.into_type(), "statue");
    assert_eq!(second.time(), Period::Constant(40));
    assert_eq!(second.placement(), MorphPlacement::Reserve);
    assert_eq!(second.cancel(), MorphCancel::Refundable);
    assert_eq!(
        second.costs(),
        [EntityCastCost::Resources(costs::cost([("gold", 30)]))]
    );
    assert!(second.requires().is_empty());
    assert_eq!(first.via_type(), None);
    assert_eq!(second.via_type(), Some("chrysalis"));
}

#[test]
fn unknown_morph_placement_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", time = 20, placement = "hover", cancel = "committed" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown morph placement");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph placement must be 'reserve', 'revalidate', or 'nearby', found 'hover'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_morph_cancel_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
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
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", time = "fast", placement = "reserve", cancel = "committed" },
            },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a morph time that is neither ticks nor a stat table");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph time must be a tick count or a { stat = ... } table, found 'fast'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_morph_time_stat_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
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

#[test]
fn parses_turret_and_its_mount() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_turret("cannon", {
            targets = GROUND,
            conduct = "on_the_move",
        })

        define_entity("wagon", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = {
                max_health = 40, speed = "0.3", radius = "1", weight = "2",
                turn_rate = 30, pivot_rate = 30, aim_rate = 30,
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            turrets = { { turret = "cannon", at = { 0, 0 }, size = { 2, 2 } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let cannon = registry.turret("cannon").expect("turret defined");
    assert_eq!(
        registry.turret_def(cannon),
        &TurretDef::new(
            Weapon::new(LayerId::new(1), Delivery::Instant, None),
            TurretStats::default(),
            WeaponConduct::OnTheMove,
        )
    );

    let expected = EntityTypeDef::new("wagon")
        .with_location(LayerId::new(1), CellSize::new(2, 2), Solidity::Solid)
        .with_movement(
            FixedU64::from_str("0.3").unwrap(),
            FixedU64::from_str("1").unwrap(),
            FixedU64::from_str("2").unwrap(),
            FixedU64::from_num(30),
            FixedU64::from_num(30),
        )
        .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(30))
        .with_health(40)
        .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(6))
        .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
        .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(6))
        .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(7))
        .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
        .with_turrets([TurretMount::new(
            cannon,
            CellPos::new(0, 0),
            CellSize::new(2, 2),
        )]);

    assert_eq!(registry.entity("wagon"), Some(&expected));
}

#[test]
fn parses_turret_reading_its_own_stats() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity_stat("flak_damage")

        define_turret("flak", {
            targets = GROUND,
            stats = { damage = "flak_damage" },
        })

        define_entity("keep", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 100, flak_damage = 3,
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            turrets = { { turret = "flak" } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let flak = registry.turret("flak").expect("turret defined");
    let reads = registry.turret_def(flak).stats();
    assert_eq!(
        reads.damage,
        registry.entity_stat("flak_damage").expect("stat defined"),
        "the gun reads the stat it named"
    );
    assert_eq!(
        reads.range,
        EntityStatId::ATTACK_RANGE,
        "and the standard one for everything it did not"
    );
}

#[test]
fn rejects_unknown_weapon_conduct() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_turret("gun", { targets = GROUND, conduct = "whenever" })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown weapon conduct");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("weapon conduct must be 'halts' or 'on_the_move', found 'whenever'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn rejects_mount_of_undefined_turret() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("wagon", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                max_health = 40,
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            turrets = { { turret = "gun" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a mount naming a turret nothing defined");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("turret 'gun' is not defined")),
        "unexpected error: {error:?}"
    );
}
