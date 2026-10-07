//! Loading content into a `ContentRegistry`: the definitions round-trip to
//! the same `EntityTypeDef`s the Rust builder produces, and malformed scripts
//! surface as errors rather than panics. The contract holds for any engine;
//! [`engine`] picks the binding the suite runs against.

mod utils;

use ferrets_content::{
    affiliation::Affiliation,
    annex::{AloneConduct, AnnexClaim, AnnexLife, AnnexWork},
    attack::{AttackDef, Delivery, Slain, Weapon},
    brood::{BroodlingDef, Lingering, OrphanFate},
    build::{BuilderAttendance, RiseStart, SitePool},
    concealment::Concealment,
    cost::Cost,
    detection::Detection,
    dying::{Bequest, DeathKind, DyingDef, LeftBy},
    entity_buffs::{EntityBuffDef, Interruption, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldAction, FieldCoverage, FieldDecay, FieldEffect, FieldGrowth, FieldLayer,
        FieldPlacement, FieldSourceDef, FieldVision,
    },
    kinds::{Kind, Kinds},
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, PoolCarry,
        RevertCarry, ViaInterrupted,
    },
    player_stats::PlayerStatId,
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    pool_shift::PoolShift,
    price::{self, Price},
    quantity::Quantity,
    repair::{RepairCost, RepairRate},
    requirement::{Bound, Requirement, Threshold},
    research::ResearchDef,
    resource::Banking,
    skills::{
        Casting, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach, SkillCaster, SkillDef,
    },
    splash::{SplashDef, SplashShape},
    stack_rule::StackRule,
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp, PlayerModifier},
    transport::{PassengerConduct, PassengerFate},
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
        .with_pool(Pool::health(40))
        .with_dying(2, [])
        .with_attack(
            AttackDef::new(Weapon::new(
                LayerId::new(1),
                Delivery::Instant,
                None,
                Slain::Remains,
            )),
            6,
            4,
            4,
            7,
            3,
        )
        .with_price([("gold", 80)])
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
                damage = 2, attack_range = 3, acquire_range = 7, attack_period = 4, damage_point = 2,
            },
            pools = { health = { maximum = 20 } },
            attack = { targets = GROUND },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("scout")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_pool(Pool::health(20))
        .with_attack(
            AttackDef::new(Weapon::new(
                LayerId::new(1),
                Delivery::Instant,
                None,
                Slain::Remains,
            )),
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
        define_entity_stat("morale", 0)
        define_entity("hero", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { morale = 7 },
            pools = { health = { maximum = 10 } },
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
fn pool_written_among_stats_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("gadget", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { health = { maximum = 20 } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a pool written among the stats");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("stat 'health' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_pool_name_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("gadget", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { mana = { maximum = 20 } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown pool");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("pool 'mana' is not defined")),
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
                damage = 12, attack_range = 1, attack_period = 4, damage_point = 2,
                armor = 4,
            },
            pools = { health = { maximum = 100 }, energy = { maximum = 50, regen = "0.5" } },
            bonus_damage_vs = { armored = 8, dragon = 15 },
            attack = { targets = GROUND },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("knight")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_pool(Pool::health(100))
        .with_armor(4)
        .with_attack(
            AttackDef::new(Weapon::new(
                LayerId::new(1),
                Delivery::Instant,
                None,
                Slain::Remains,
            )),
            12,
            1,
            1,
            4,
            2,
        )
        .with_bonus_damage_vs([("armored", 8u32), ("dragon", 15u32)])
        .with_pool(Pool::builtin(
            PoolId::ENERGY,
            FixedU64::from_num(50),
            FixedU64::from_str("0.5").unwrap(),
            FixedU64::ZERO,
            PoolInitial::Full,
        ));

    assert_eq!(registry.entity("knight"), Some(&expected));
}

#[test]
fn pool_reads_initial() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("lamp", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = {
                health = { maximum = 20, initial = "full" },
                energy = { maximum = 10, initial = "2.5" },
            },
        })
        define_entity("wick", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = {
                health = { maximum = 20 },
                energy = { maximum = 200, initial = { share = "0.25" } },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let initial = |name: &str, pool: PoolId| {
        registry
            .entity(name)
            .and_then(|def| def.base_stats.pool(pool))
            .map(|pool| pool.initial())
    };
    assert_eq!(initial("lamp", PoolId::HEALTH), Some(PoolInitial::Full));
    assert_eq!(
        initial("lamp", PoolId::ENERGY),
        Some(PoolInitial::Amount(FixedU64::from_str("2.5").unwrap()))
    );
    assert_eq!(initial("wick", PoolId::HEALTH), Some(PoolInitial::Full));
    assert_eq!(
        initial("wick", PoolId::ENERGY),
        Some(PoolInitial::Share(FixedU64::from_str("0.25").unwrap()))
    );
}

#[test]
fn build_reads_time_and_how_site_holds_each_pool() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("tower", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = {
                health = { maximum = 400 },
                energy = { maximum = 200, initial = 50 },
            },
            build = { time = 10, pools = { health = { rises_from = { share = "0.1" } }, energy = "withheld" } },
        })
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = "initial" } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let tower = registry
        .entity("tower")
        .and_then(|def| def.build.as_ref())
        .expect("the tower is built");
    assert_eq!(tower.time(), 10);
    assert_eq!(
        tower.site(PoolId::HEALTH),
        Some(SitePool::Rising(RiseStart::Share(
            FixedU64::from_str("0.1").unwrap()
        )))
    );
    assert_eq!(tower.site(PoolId::ENERGY), Some(SitePool::Withheld));
    let hut = registry
        .entity("hut")
        .and_then(|def| def.build.as_ref())
        .expect("the hut is built");
    assert_eq!(hut.time(), 4);
    assert_eq!(hut.site(PoolId::HEALTH), Some(SitePool::Initial));
}

#[test]
fn build_pool_of_unknown_word_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = "rising" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("an unknown site word is refused");
    };
    assert_eq!(
        error.to_string(),
        "content error: build pool 'health' must be 'initial', 'withheld', or a { rises_from = ... } table, found 'rising'"
    );
}

#[test]
fn build_pool_rising_without_start_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = {} } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a rising pool names where it rises from");
    };
    assert_eq!(
        error.to_string(),
        "content error: build pool 'health' rises_from must be a non-negative integer or a decimal string, got nil"
    );
}

#[test]
fn build_pool_rising_from_full_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = { rises_from = "full" } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a pool rises from below full");
    };
    assert_eq!(
        error.to_string(),
        "invalid number: 'full': invalid digit found in string"
    );
}

#[test]
fn build_pool_of_unknown_name_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = "initial", mana = "initial" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("the build table is refused");
    };
    assert_eq!(
        error.to_string(),
        "content error: pool 'mana' is not defined"
    );
}

#[test]
fn build_pool_rising_from_table_without_share_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4, pools = { health = { rises_from = { shar = "0.1" } } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("the build table is refused");
    };
    assert_eq!(
        error.to_string(),
        "content error: build pool 'health' rises_from share must be a non-negative integer or a decimal string, got nil"
    );
}

#[test]
fn build_without_time_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { pools = { health = "initial" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("the build table is refused");
    };
    assert_eq!(
        error.to_string(),
        "content error: field 'time': error converting Lua nil to u32 (expected number or string coercible to number)"
    );
}

#[test]
fn build_without_pools_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("hut", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            build = { time = 4 },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("the build table is refused");
    };
    assert_eq!(
        error.to_string(),
        "content error: field 'pools': error converting Lua nil to table"
    );
}

#[test]
fn pool_initial_of_unknown_word_is_rejected() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("lamp", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 20, initial = "half" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("an unknown initial word is refused");
    };
    assert_eq!(
        error.to_string(),
        "invalid number: 'half': invalid digit found in string"
    );
}

#[test]
fn parses_repairer_and_repair_ratio() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_resource("gold")
        define_tag("building")

        define_entity("depot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            price = { gold = 200 },
            build = { time = 20, pools = { health = "initial" } },
            repair_ratio = "0.5",
            tags = { "building" },
        })

        define_entity("worker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { repair_speed = "1.0", repair_cost_factor = "0.25",
                repair_range = 1,
            },
            pools = { health = { maximum = 20 } },
            repairer = {
                repairs = { tags = { "building" } },
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
    assert_eq!(repairer.repairs(), &Kinds::tags(["building"]));
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
            stats = { speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", cargo_size = 1 },
            pools = { health = { maximum = 60 } },
            tags = { "infantry" },
        })

        define_entity("wagon", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = "0.25", turn_rate = 30, pivot_rate = 30, radius = "0.5", cargo_capacity = 6,
                load_range = 2, unload_range = 3, load_period = 4, unload_period = 8,
            },
            pools = { health = { maximum = 150 } },
            transporter = {
                carries = { types = { "footman" }, tags = { "infantry" } },
                boarding = "allied",
                fate = "eject",
                conduct = "fight",
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let wagon = registry.entity("wagon").expect("wagon defined");
    let transporter = wagon.transporter.as_ref().expect("wagon can transport");
    assert_eq!(
        transporter.carries(),
        &Kinds::only([
            Kind::Type("footman".to_string()),
            Kind::Tag("infantry".to_string()),
        ])
    );
    assert_eq!(transporter.boarding(), Affiliation::Allied);
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
            stats = { cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            pools = { health = { maximum = 100 } },
            transporter = { carries = { tags = { "infantry" } }, boarding = "own", fate = "destroy", conduct = "shelter" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let transporter = registry
        .entity("cart")
        .expect("cart defined")
        .transporter
        .as_ref()
        .expect("cart can transport");
    assert_eq!(transporter.boarding(), Affiliation::Own);
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
            stats = { cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            pools = { health = { maximum = 100 } },
            transporter = { carries = { tags = { "infantry" } }, boarding = "own", fate = "destroy", conduct = "mutiny" },
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
fn unknown_boarding_affiliation_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_tag("infantry")

        define_entity("cart", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            pools = { health = { maximum = 100 } },
            transporter = { carries = { tags = { "infantry" } }, boarding = "everybody", fate = "destroy", conduct = "shelter" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a boarding rule naming nobody the engine knows");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("affiliation must be 'own', 'allied', 'enemy', or 'anyone', found 'everybody'")),
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
            stats = { cargo_capacity = 2,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
            },
            pools = { health = { maximum = 100 } },
            transporter = { carries = { tags = { "infantry" } }, boarding = "own", fate = "scatter", conduct = "shelter" },
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
            stats = { repair_speed = "1.0", repair_range = 1 },
            pools = { health = { maximum = 20 } },
            repairer = {
                repairs = { tags = { "building" } },
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
        &RepairCost::PerTick(price::from([("gold", 2u32)]))
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
                repair_speed = "1.0", repair_range = 2,
            },
            pools = { health = { maximum = 45 }, energy = { maximum = 200, regen = "0.2" } },
            repairer = {
                repairs = { tags = { "biological" } },
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
            lasting = { ticks = 20 },
            stack = "refresh",
            effects = { { stats = {
                { entity_stat = "damage", op = "percent", value = "1.0" },
            } } },
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
            pools = { health = { maximum = 40 }, energy = { maximum = 100, regen = "1" } },
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
                costs: vec![Cost::Energy(FixedU64::from_num(30))],
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
            lasting = { ticks = 20 },
            stack = "refresh",
            effects = { { stats = {
                { entity_stat = "damage", op = "percent", value = "1.0" },
            } } },
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
            pools = { health = { maximum = 40 } },
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
            entity_modifiers = { { stats = { { entity_stat = "speed", op = "percent", value = "0.5" } } } },
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
            pools = { health = { maximum = 10 } },
            requires = { { forge = "blacksmith" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry naming no kind");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains(
            "a requirement names exactly one of all, any, unless, entity_type, tag, research, annexed, health, energy, stat, idle_for, or unhurt_for"
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
            pools = { health = { maximum = 10 } },
            requires = { { entity_type = "blacksmith", tag = "workshop" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry naming two kinds");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains(
            "a requirement names exactly one of all, any, unless, entity_type, tag, research, annexed, health, energy, stat, idle_for, or unhurt_for"
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
            pools = { health = { maximum = 10 } },
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
            "a requirement names exactly one of all, any, unless, entity_type, tag, research, annexed, health, energy, stat, idle_for, or unhurt_for"
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
            pools = { health = { maximum = 10 } },
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
            entity_modifiers = { { stats = { { entity_stat = "speed", op = "percent", value = "0.5" } } } },
        })

        define_skill("war_cry", {
            caster = "player",
            cooldown = 30,
            price = { gold = 25 },
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
                price: price::from([("gold", 25)]),
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
        matches!(&error, ScriptError::ContentError(m) if m.contains("skill target must be 'caster', 'position', 'fallen', 'own', 'allied', 'enemy', or 'anyone', found 'everyone'")),
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
            entity_modifiers = { { stats = { { entity_stat = "speed", op = "percent", value = "0.5" } } } },
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
        vec![EntityModifiers::Stats(vec![EntityModifier {
            stat: EntityStatId::SPEED,
            op: ModifierOp::PercentAdd,
            magnitude: FixedI64::from_num(0.5),
        }])]
    );
}

#[test]
fn player_stat_in_entity_modifier_list_errors() {
    let source = r#"
        define_entity_buff("confused", {
            lasting = { ticks = 10 },
            stack = "refresh",
            effects = { { stats = {
                { player_stat = "max_supply", op = "flat", value = "1" },
            } } },
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
                damage = 12, attack_range = 6, attack_period = 10, damage_point = 4,
            },
            pools = { health = { maximum = 30 } },
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
        .with_pool(Pool::health(30))
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
                Slain::Remains,
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
            stats = { sight_range = 12 },
            pools = { health = { maximum = 20 } },
            selection = { priority = 42, class = "spellcaster" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let expected = EntityTypeDef::new("caster")
        .with_location(LayerId::new(1), CellSize::ONE, Solidity::Solid)
        .with_pool(Pool::health(20))
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
            .any(|b| b == "town_hall"),
        "a builds list carries every name it declares"
    );

    let hall = registry.entity("town_hall").expect("town_hall");
    assert!(
        hall.trainer
            .as_ref()
            .unwrap()
            .trains()
            .any(|t| t == "peasant"),
        "and a trains list the same"
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
            entity_modifiers = { { stats = { { entity_stat = "damage", op = "flat", value = "5" } } } },
        })
        define_research("smithing", {
            price = { gold = 30 },
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
        price::from([("gold", 30)]),
        200,
        registry.player_buff("sharp_blades"),
        [Requirement::EntityType("lab".to_string())],
    );
    assert_eq!(registry.research_def(smithing), Some(&expected));

    // An omitted cost is free, an omitted buff a pure unlock.
    let tactics = registry.research("tactics").expect("tactics registered");
    let expected = ResearchDef::new(Price::new(), 100, None, Vec::new());
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
fn loads_requirement_nodes_and_state_leaves() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("keep", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("castle", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_research("tactics", { time = 10 })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { any = {
                { entity_type = "keep" },
                { all = {
                    { entity_type = "castle" },
                    { research = "tactics" },
                    { health = { under_share = "0.34" } },
                    { energy = { at_least = 20 } },
                    { stat = "speed", under = "0.2" },
                    { stat = "speed", at_least_share = "1.5" },
                    "idle",
                    { idle_for = 40 },
                    { unhurt_for = 200 },
                } },
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let tactics = registry.research("tactics").expect("tactics registered");
    assert_eq!(
        registry.entity("knight").unwrap().requires,
        vec![Requirement::Any(vec![
            Requirement::EntityType("keep".to_string()),
            Requirement::All(vec![
                Requirement::EntityType("castle".to_string()),
                Requirement::Research(tactics),
                Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.34")))),
                Requirement::Energy(Bound::Amount(Threshold::AtLeast(FixedU64::from_num(20)))),
                Requirement::Stat {
                    stat: EntityStatId::SPEED,
                    bound: Bound::Amount(Threshold::Under(utils::fixed("0.2"))),
                },
                Requirement::Stat {
                    stat: EntityStatId::SPEED,
                    bound: Bound::Share(Threshold::AtLeast(utils::fixed("1.5"))),
                },
                Requirement::Idle,
                Requirement::IdleFor(40),
                Requirement::UnhurtFor(200),
            ]),
        ])]
    );
}

#[test]
fn requirement_table_mixing_list_and_key_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_tag("hall")
        define_entity("forge", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            tags = { "hall" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { entity_type = "forge" }, any = { { tag = "hall" } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a table that lists requirements and names one");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a requirement table names a requirement or lists requirements, not both")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_list_with_stray_key_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_research("masonry", { time = 10 })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { reserch = "masonry" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a misspelled requirement key");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("requires lists requirements and also names 'reserch', which is none of them")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_node_list_naming_key_beside_entries_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("keep", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { any = { { entity_type = "keep" }, entity_type = "castle" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a requirement key written beside a node's entries");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a requirement table names a requirement or lists requirements, not both")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn bare_list_inside_requirement_list_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_tag("hall")
        define_entity("forge", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            tags = { "hall" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { entity_type = "forge" }, { { tag = "hall" } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a bare list nested in a requirement list");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a list of requirements inside a list is written as { all = { ... } } or { any = { ... } }")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_list_with_several_stray_keys_names_first_in_sorted_order() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("keep", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { entity_type = "keep" }, zeal = 1, banner = 2, moat = 3 },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject stray keys in a requirement list");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("requires lists requirements and also names 'banner', which is none of them")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_list_with_hole_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_research("masonry", { time = 10 })
        define_entity("keep", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { entity_type = "keep" }, UNDEFINED, { research = "masonry" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a requirement list with a hole");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("requires has no requirement at 2")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_list_with_index_past_its_length_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("keep", {
            location = { occupation = ground, size = 1, solidity = "solid" },
        })
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { [1] = { entity_type = "keep" }, [3] = { entity_type = "keep" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an entry past the list's length");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("requires lists requirements and also names [3], which is none of them")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn as_long_as_list_with_stray_key_errors() {
    let source = r#"
        define_entity_buff("hidden", {
            effects = {},
            lasting = { as_long_as = { "idle", unhurt_fr = 40 } },
            stack = "ignore",
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a stray key in an as_long_as list");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("as_long_as lists requirements and also names 'unhurt_fr', which is none of them")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn requirement_string_other_than_idle_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { "resting" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a requirement string that is not 'idle'");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a requirement must be 'built', 'idle', an { all = ... } or { any = ... } table, or a table naming one of unless, entity_type, tag, research, annexed, health, energy, stat, idle_for, or unhurt_for, found 'resting'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn as_long_as_reads_built() {
    let source = r#"
        define_entity_buff("finished", {
            lasting = { as_long_as = { "built", "idle" } },
            stack = "ignore",
            effects = { "conceal" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let finished = registry
        .entity_buff("finished")
        .expect("finished registered");
    assert_eq!(
        registry.entity_buff_def(finished).lasting,
        Lasting::While(Requirement::All(vec![
            Requirement::Built,
            Requirement::Idle
        ]))
    );
}

#[test]
fn as_long_as_reads_unless() {
    let source = r#"
        define_entity_buff("scaffolding", {
            lasting = { as_long_as = { unless = "built" } },
            stack = "ignore",
            effects = { "conceal" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");
    let scaffolding = registry
        .entity_buff("scaffolding")
        .expect("scaffolding registered");
    assert_eq!(
        registry.entity_buff_def(scaffolding).lasting,
        Lasting::While(Requirement::Unless(Box::new(Requirement::Built)))
    );
}

#[test]
fn as_long_as_reads_bare_list_as_all() {
    let source = r#"
        define_entity_buff("steady", {
            lasting = { as_long_as = { "idle", { unhurt_for = 40 } } },
            stack = "ignore",
            effects = { "conceal" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let steady = registry.entity_buff("steady").expect("steady registered");
    assert_eq!(
        registry.entity_buff_def(steady).lasting,
        Lasting::While(Requirement::All(vec![
            Requirement::Idle,
            Requirement::UnhurtFor(40),
        ]))
    );
}

#[test]
fn bound_naming_both_sides_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { health = { under = 20, under_share = "0.5" } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a bound naming both sides");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a health requirement names exactly one of under, at_least, under_share, or at_least_share")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn stat_requirement_with_unknown_stat_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("knight", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            requires = { { stat = "girth", under = "1" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unregistered stat");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a requirement names the stat 'girth', which is not registered")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn loads_while_buff_and_passives() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity_buff("on_fire", {
            lasting = { as_long_as = { health = { under_share = "0.34" } } },
            stack = "ignore",
            effects = { { stats = { { entity_stat = "health_drain", op = "flat", value = "0.15" } } } },
        })
        define_entity("depot", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            pools = { health = { maximum = 200 } },
            passives = { "on_fire" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let on_fire = registry.entity_buff("on_fire").expect("on_fire registered");
    let def = registry.entity_buff_def(on_fire);
    assert_eq!(
        def.lasting,
        Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
            utils::fixed("0.34")
        ))))
    );
    assert_eq!(registry.entity("depot").unwrap().passives, vec![on_fire]);
}

#[test]
fn passive_naming_unknown_buff_errors() {
    let source = r#"
        local ground = define_layer("ground")
        define_entity("depot", {
            location = { occupation = ground, size = 1, solidity = "solid" },
            passives = { "on_fire" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown passive");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("entity buff 'on_fire' is not defined")),
        "unexpected error: {error:?}"
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
            stats = { repair_speed = "1.0", repair_range = 1 },
            pools = { health = { maximum = 20 } },
            repairer = { repairs = { tags = { "building" } }, presence = { present = { crew = 1 } } },
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
            stats = { repair_speed = "1.0", repair_range = 1 },
            pools = { health = { maximum = 20 } },
            repairer = {
                repairs = { tags = { "building" } },
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
            stats = { repair_speed = "1.0", repair_range = 1 },
            pools = { health = { maximum = 20 } },
            repairer = {
                repairs = { tags = { "biological" } },
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
            stats = { repair_speed = "1.0" },
            pools = { health = { maximum = 20 } },
            repairer = {
                repairs = { tags = { "building" } },
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
            stats = { build_range = 1 },
            pools = { health = { maximum = 100 } },
            tags = { "building" },
            builder = { builds = { "tech_lab" }, attendance = { present = { crew = 1 } } },
            docks = { { at = { 2, 0 }, accepts = { types = { "tech_lab" } } } },
        })
        define_entity("tech_lab", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 40, drain = "2" } },
            tags = { "building" },
            price = { gold = 25 },
            build = { time = 10, pools = { health = "initial" } },
            annex = { alone = { work = "idles", life = { fades = "2" } }, claim = "seized" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    let barracks = registry.entity("barracks").expect("barracks defined");
    let dock = barracks.docks.first().expect("it offers one dock");
    assert_eq!(dock.at(), CellPos::new(2, 0));
    let takes = |name: &str| {
        dock.accepts()
            .admits(registry.entity(name).expect("type is registered"))
    };
    assert!(
        takes("tech_lab") && !takes("barracks"),
        "a dock takes the annex it names and nothing else"
    );

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
            duration: 40,
            detection: Detection::Blind,
        }
    );
}

#[test]
fn skill_reads_time_it_is_worked_over() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity_stat("ritual_time", 1)

        define_skill("bolt", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            effect = { damage = "5" },
        })
        define_skill("raise_dead", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            cast = { point = 30, period = 45 },
            effect = { damage = "5" },
        })
        define_skill("ritual", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            cast = { point = { stat = "ritual_time" } },
            effect = { damage = "5" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let ritual_time = registry
        .entity_stat("ritual_time")
        .expect("the stat is registered");
    let casting = |name: &str| {
        let def = registry
            .skill(name)
            .and_then(|id| registry.skill_def(id))
            .expect("the skill is defined");
        let SkillCaster::Entity { casting, .. } = &def.caster else {
            panic!("{name} is cast by an entity");
        };
        *casting
    };

    assert_eq!(
        casting("bolt"),
        Casting::Instant,
        "a skill saying nothing about time lands the tick it is cast"
    );
    assert_eq!(
        casting("raise_dead"),
        Casting::Delayed {
            point: Quantity::Constant(30),
            period: Quantity::Constant(45),
        }
    );
    assert_eq!(
        casting("ritual"),
        Casting::Delayed {
            point: Quantity::Stat(ritual_time),
            period: Quantity::Stat(ritual_time),
        },
        "an omitted period is the point itself: the caster is free the tick it lands"
    );
}

#[test]
fn entity_stat_without_floor_is_refused() {
    let source = r#"
        define_entity_stat("morale")
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("a stat's floor is not optional");
    };

    assert!(
        format!("{error}").contains("entity stat floor"),
        "a stat declared without a floor is refused where it is declared: {error}"
    );
}

#[test]
fn skill_reads_reach_it_is_cast_from() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity_stat("raise_range", 1)

        define_skill("bolt", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            effect = { damage = "5" },
        })
        define_skill("raise_dead", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            range = 6,
            effect = { damage = "5" },
        })
        define_skill("far_sight", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            range = { stat = "raise_range" },
            effect = { damage = "5" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let raise_range = registry
        .entity_stat("raise_range")
        .expect("the stat is registered");
    let reach = |name: &str| {
        let def = registry
            .skill(name)
            .and_then(|id| registry.skill_def(id))
            .expect("the skill is defined");
        let SkillCaster::Entity { reach, .. } = &def.caster else {
            panic!("{name} is cast by an entity");
        };
        *reach
    };

    assert_eq!(
        reach("bolt"),
        Reach::Wherever,
        "a skill that names no range is cast where the caster stands"
    );
    assert_eq!(reach("raise_dead"), Reach::Within(Quantity::Constant(6)));
    assert_eq!(
        reach("far_sight"),
        Reach::Within(Quantity::Stat(raise_range))
    );
}

#[test]
fn skill_reach_by_undefined_stat_errors() {
    let source = r#"
        define_layer("ground")

        define_skill("far_sight", {
            cooldown = 10,
            caster = "entity",
            target = "enemy",
            range = { stat = "raise_range" },
            effect = { damage = "5" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a reach read from a stat that is not defined");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("cast range stat 'raise_range' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn summon_effect_reads_its_type_and_count() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("skeleton", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 20 } },
        })
        define_skill("raise_dead", {
            cooldown = 10,
            caster = "entity",
            target = "fallen",
            effect = { summon = { entity = "skeleton", count = 2 } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let skeleton = registry.type_id("skeleton").expect("skeleton defined");
    let skill = registry
        .skill("raise_dead")
        .and_then(|id| registry.skill_def(id))
        .expect("raise_dead defined");
    let SkillCaster::Entity { target, effect, .. } = &skill.caster else {
        panic!("raise_dead is cast by an entity");
    };
    assert_eq!(*target, EntityCastTarget::Fallen { kinds: Kinds::Any });
    assert_eq!(
        *effect,
        EntityCastEffect::Summon {
            entity_type: skeleton,
            count: 2,
        }
    );
}

#[test]
fn summon_of_undefined_type_errors() {
    let source = r#"
        define_layer("ground")

        define_skill("raise_dead", {
            cooldown = 10,
            caster = "entity",
            target = "fallen",
            effect = { summon = { entity = "skeleton", count = 2 } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a summon of a type that is not defined");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("entity type 'skeleton' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
#[should_panic(expected = "a death that waits for nothing and leaves nothing is no dying at all")]
fn empty_dying_block_panics_on_load() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("marine", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 40 } },
            dying = {},
        })
    "#;
    let _ = content::load(&engine(), source);
}

#[test]
fn dying_reads_what_it_leaves_and_deaths_that_leave_it() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("corpse", {
            location = { occupation = GROUND, size = 1, solidity = "passable" },
            tags = { "remains" },
            stats = { lifetime = 600 },
        })
        define_entity("broodling", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 20 } },
        })
        -- A body states what it rots into and no time: it has lain its whole
        -- life already, so what it leaves goes down the tick its decay ends.
        define_entity("ash", {
            location = { occupation = GROUND, size = 1, solidity = "passable" },
            tags = { "remains" },
            stats = { lifetime = 60 },
            dying = { leaves = { { entity = "corpse" } } },
        })
        -- An entry that names no deaths takes the engine's own rule; one that
        -- names them is left by exactly those. A count of one is the default.
        define_entity("marine", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 40 } },
            dying = { time = 2, leaves = { { entity = "corpse" } } },
        })
        define_entity("hive", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            pools = { health = { maximum = 200 } },
            dying = { time = 2, leaves = {
                { entity = "corpse", on = { "killed", "expired" } },
                { entity = "broodling", count = 2, on = { "killed" } },
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let leaves = |name: &str| {
        registry
            .entity(name)
            .and_then(|def| def.dying.as_ref())
            .map(DyingDef::leaves)
            .expect("the type has a dying phase")
            .to_vec()
    };

    let dying_time = |name: &str| {
        registry
            .entity(name)
            .and_then(|def| def.dying.as_ref())
            .expect("the type has a dying phase")
            .dying_time()
    };
    assert_eq!(dying_time("marine"), Some(2));
    assert_eq!(
        dying_time("ash"),
        None,
        "a body states what it rots into and no wait before it goes"
    );
    assert_eq!(
        leaves("marine"),
        vec![Bequest::new("corpse", 1, LeftBy::Ordinary)]
    );
    assert_eq!(
        leaves("hive"),
        vec![
            Bequest::new(
                "corpse",
                1,
                LeftBy::Named(vec![DeathKind::Killed, DeathKind::Expired])
            ),
            Bequest::new("broodling", 2, LeftBy::Named(vec![DeathKind::Killed])),
        ]
    );
    assert!(
        registry
            .entity("corpse")
            .expect("the type is registered")
            .is_remains(),
        "the engine's own remains tag is what makes a body a body"
    );
}

#[test]
fn weapon_reads_what_it_leaves_of_what_it_kills() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("mortar", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { damage = 10, attack_range = 6,
                acquire_range = 8, attack_period = 20, damage_point = 8,
            },
            pools = { health = { maximum = 30 } },
            attack = { targets = GROUND, slain = "nothing" },
        })
        define_entity("marine", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { damage = 10, attack_range = 6,
                acquire_range = 8, attack_period = 20, damage_point = 8,
            },
            pools = { health = { maximum = 30 } },
            attack = { targets = GROUND },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let slain = |name: &str| {
        registry
            .entity(name)
            .and_then(|def| def.attack.as_ref())
            .expect("the type points a weapon")
            .weapon()
            .slain()
    };

    assert_eq!(slain("mortar"), Slain::Nothing);
    assert_eq!(
        slain("marine"),
        Slain::Remains,
        "a weapon that says nothing leaves whatever its victim leaves"
    );
}

#[test]
fn lifetime_stat_parses() {
    let source = r#"
        local GROUND = define_layer("ground")

        define_entity("skeleton", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { lifetime = 900 },
            pools = { health = { maximum = 50 } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");

    assert_eq!(
        registry
            .entity("skeleton")
            .and_then(|def| def.base_stat(EntityStatId::LIFETIME)),
        Some(FixedU64::from_num(900))
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
            pools = { health = { maximum = 100 } },
            resource_source = { kind = "gold", depletion = "persist" },
        })
        define_entity("tapper", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { harvest_range = 1 },
            pools = { health = { maximum = 20 } },
            resource_carrier = {
                gold = {
                    capacity = 5, time = 20,
                    presence = { hidden = { crew = 3 } },
                    sources = { types = { "refinery" } },
                },
            },
        })
        define_entity("gang", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { harvest_range = 1 },
            pools = { health = { maximum = 20 } },
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
    assert_eq!(gold.sources(), &Kinds::types(["refinery"]));
    let source = |name: &str| registry.entity(name).expect("source is registered");
    assert!(
        !gold.sources().admits(source("seam")),
        "a named source shuts out every other, bare seam included"
    );

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
    assert!(
        gang.sources().admits(source("seam")) && gang.sources().admits(source("refinery")),
        "naming no source takes every one of that kind"
    );
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
            stats = { harvest_range = 1 },
            pools = { health = { maximum = 20 } },
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
            pools = { health = { maximum = 100 } },
            price = { gold = 10 },
            build = { time = 4, pools = { health = "initial" } },
            berths = {
                rim = { points = { { 1, 0 }, { "1.8", "1.0" }, { 1, "1.8" }, { "0.2", 1 } }, slots = 2 },
                ledge = { points = { { 0, 0 }, { 1, 1 }, { 0, 1 } } },
            },
        })
        define_entity("sprite", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { harvest_range = 1, build_range = 1 },
            pools = { health = { maximum = 20 } },
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
            pools = { health = { maximum = 100 } },
            price = { gold = 10 },
            build = { time = 4, pools = { health = "initial" } },
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
            stats = { harvest_range = 1 },
            pools = { health = { maximum = 20 } },
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
fn berth_point_that_is_not_pair_errors() {
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
            stats = { harvest_range = 1 },
            pools = { health = { maximum = 20 } },
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
        FieldLayer::Passable(LayerMask::from(LayerId::new(1)))
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
            Emission::Held(1),
            Emission::Full
        )]
    );
    let pylon = registry.entity("pylon").unwrap();
    assert_eq!(
        pylon.field_sources,
        vec![FieldSourceDef::new(
            power,
            6,
            FieldGrowth::Instant,
            Emission::Nothing,
            Emission::Nothing
        )]
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
                of: Affiliation::Own,
                coverage: FieldCoverage::Any,
            },
            FieldPlacement::Forbids { field: creep },
        ]
    );
    assert_eq!(
        gateway.field_effects,
        vec![FieldEffect::new(
            power,
            Affiliation::Own,
            FieldCoverage::Every,
            Vec::new(),
            vec![EntityEffect::Disable],
            None
        )]
    );
    assert_eq!(
        registry.entity("zergling").unwrap().field_effects,
        vec![FieldEffect::new(
            creep,
            Affiliation::Anyone,
            FieldCoverage::Any,
            vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_str("0.3").unwrap(),
                }
            ]))],
            Vec::new(),
            None
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
            field_sources = { { field = "creep", radius = 3, growth = "instant", while_constructing = "nothing", while_disabled = "full" } },
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
            field_placement = { { requires = "creep", forbids = "creep", of = "anyone", coverage = "any" } },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("two verbs");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("exactly one of requires or forbids")),
        "{error:?}"
    );
}

#[test]
fn unknown_field_coverage_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("veil", { layer = "anywhere", decay = "instant" })
        define_entity("zealot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_effects = { { field = "veil", of = "allied", coverage = "half", inside = { "conceal" } } },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad coverage");
    assert!(
        matches!(&error, ScriptError::ContentError(message) if message.contains("field coverage must be 'every' or 'any', found 'half'")),
        "{error:?}"
    );
}

#[test]
fn field_effect_without_coverage_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("veil", { layer = "anywhere", decay = "instant" })
        define_entity("zealot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_effects = { { field = "veil", of = "allied", inside = { "conceal" } } },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("missing coverage");
    assert!(
        matches!(
            &error,
            ScriptError::ContentError(message)
                if message == "field 'coverage': error converting Lua nil to String (expected string or number)"
        ),
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
        define_entity_stat("brood_period", 1)
        define_entity("hatch", {
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { brood_period = 12 },
            pools = { health = { maximum = 300 } },
            berths = { brood = { points = { { "0.5", "3.5" }, { "-1", "1.0" } }, slots = 2 } },
            breeder = { breeds = "grub", period = { stat = "brood_period" }, limit = 2, initial = 1,
                        orphans = { linger = { reseat = { distance = 3 } } } },
        })
        define_entity("grub", {
            location = { occupation = GROUND, size = 1, solidity = "passable" },
            pools = { health = { maximum = 25 } },
            broodling = { berths = "brood", stance = "still" },
        })
        define_entity("pen", {
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            berths = { sty = { points = { { "0.5", "2.5" }, { "1.5", "2.5" }, { "0.5", "-0.5" }, { "1.5", "-0.5" } }, slots = 4 } },
            breeder = { breeds = "piglet", period = 30, limit = 4, orphans = "perish" },
        })
        define_entity("piglet", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 20 } },
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
    assert_eq!(brood.period(), Quantity::Stat(brood_period));
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
    assert_eq!(brood.period(), Quantity::Constant(30));
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
            pools = { health = { maximum = 300 } },
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
            tags = { "winged" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", land_pool_carry = { health = "share" }, time = 20, placement = "reserve", cancel = "committed", reason = "growth" },
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
            pools = { health = { maximum = 300 } },
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
            pools = { health = { maximum = 300 } },
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
            pools = { health = { maximum = 40 } },
            morphs = {
                { into = "flier", land_pool_carry = { health = "full" }, time = 20, placement = "nearby", cancel = "refundable",
                  interrupted = "dies", reason = "production" },
                { into = "statue", land_pool_carry = { health = "share" }, time = 20, placement = "revalidate", cancel = "committed" },
            },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            pools = { health = { maximum = 40 } },
        })
        define_entity("statue", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let walker = registry.entity("walker").expect("walker is registered");
    let [first, second] = walker.morphs.as_slice() else {
        panic!("walker declares exactly two transitions");
    };
    assert_eq!(first.placement(), MorphPlacement::Nearby);
    assert_eq!(first.course(), &MorphCourse::direct(MorphInterrupted::Dies));
    assert_eq!(first.reason(), MorphReason::Production);
    assert_eq!(
        second.course(),
        &MorphCourse::direct(MorphInterrupted::Reverts)
    );
    assert_eq!(second.reason(), MorphReason::Change);
    assert_eq!(
        first.land_pool_carry().collect::<Vec<_>>(),
        [(PoolId::HEALTH, PoolCarry::Full)]
    );
    assert_eq!(
        second.land_pool_carry().collect::<Vec<_>>(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))]
    );
}

#[test]
fn morph_without_pool_carry_names_no_pool() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = { { into = "flier", time = 20, placement = "reserve", cancel = "committed" } },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let walker = registry.entity("walker").expect("walker is registered");
    assert_eq!(walker.morphs[0].land_pool_carry().count(), 0);
}

#[test]
fn pool_carry_reads_each_keyword() {
    let source = r#"
        local GROUND = define_layer("ground")
        local function form(name)
            define_entity(name, {
                location = { occupation = GROUND, size = 1, solidity = "solid" },
                pools = { health = { maximum = 10 } },
            })
        end
        form("shared")
        form("differed")
        form("clamped")
        form("full")
        form("started")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 10 } },
            morphs = {
                { into = "shared", land_pool_carry = { health = "share" }, time = 20, placement = "reserve", cancel = "committed" },
                { into = "differed", land_pool_carry = { health = "difference" }, time = 20, placement = "reserve", cancel = "committed" },
                { into = "clamped", land_pool_carry = { health = "clamp" }, time = 20, placement = "reserve", cancel = "committed" },
                { into = "full", land_pool_carry = { health = "full" }, time = 20, placement = "reserve", cancel = "committed" },
                { into = "started", land_pool_carry = { health = "initial" }, time = 20, placement = "reserve", cancel = "committed" },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let carries: Vec<PoolCarry> = registry
        .entity("walker")
        .unwrap()
        .morphs
        .iter()
        .flat_map(|morph| morph.land_pool_carry().map(|(_, carry)| carry))
        .collect();
    assert_eq!(
        carries,
        vec![
            PoolCarry::Shift(PoolShift::Share),
            PoolCarry::Shift(PoolShift::Difference),
            PoolCarry::Shift(PoolShift::Clamp),
            PoolCarry::Full,
            PoolCarry::Initial,
        ]
    );
}

#[test]
fn unknown_pool_carry_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = { { into = "flier", land_pool_carry = { health = "half" }, time = 20, placement = "reserve", cancel = "committed" } },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown pool carry");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("pool carry must be 'share', 'difference', 'clamp', 'full', or 'initial', found 'half'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn via_reads_form_entering_carries_and_reverts() {
    let source = r#"
        local GROUND = define_layer("ground")
        local function form(name)
            define_entity(name, {
                location = { occupation = GROUND, size = 1, solidity = "solid" },
                pools = { health = { maximum = 10 }, energy = { maximum = 10 } },
            })
        end
        form("cocoon")
        form("moth")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 10 }, energy = { maximum = 10 } },
            morphs = { {
                into = "moth",
                via = {
                    form = "cocoon",
                    enter_pool_carry = { health = "full", energy = "difference" },
                    interrupted = { reverts = { health = "restore", energy = "clamp" } },
                },
                land_pool_carry = { health = "share" },
                time = 20, placement = "reserve", cancel = "committed",
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let larva = registry.entity("larva").expect("larva is registered");
    assert_eq!(
        larva.morphs[0].course(),
        &MorphCourse::via(
            "cocoon",
            [
                (PoolId::HEALTH, PoolCarry::Full),
                (PoolId::ENERGY, PoolCarry::Shift(PoolShift::Difference)),
            ],
            ViaInterrupted::reverts([
                (PoolId::HEALTH, RevertCarry::Restore),
                (
                    PoolId::ENERGY,
                    RevertCarry::Carry(PoolCarry::Shift(PoolShift::Clamp)),
                ),
            ]),
        )
    );
}

#[test]
fn via_reads_initial_on_entering_and_reverting() {
    let source = r#"
        local GROUND = define_layer("ground")
        local function form(name)
            define_entity(name, {
                location = { occupation = GROUND, size = 1, solidity = "solid" },
                pools = { health = { maximum = 10 }, energy = { maximum = 10 } },
            })
        end
        form("cocoon")
        form("moth")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 10 }, energy = { maximum = 10 } },
            morphs = { {
                into = "moth",
                via = {
                    form = "cocoon",
                    enter_pool_carry = { energy = "initial" },
                    interrupted = { reverts = { energy = "initial" } },
                },
                time = 20, placement = "reserve", cancel = "committed",
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let larva = registry.entity("larva").expect("larva is registered");
    assert_eq!(
        larva.morphs[0].course(),
        &MorphCourse::via(
            "cocoon",
            [(PoolId::ENERGY, PoolCarry::Initial)],
            ViaInterrupted::reverts([(PoolId::ENERGY, RevertCarry::Carry(PoolCarry::Initial))]),
        )
    );
}

#[test]
fn via_without_carries_names_no_pool_and_reads_dies() {
    let source = r#"
        local GROUND = define_layer("ground")
        local function form(name)
            define_entity(name, {
                location = { occupation = GROUND, size = 1, solidity = "solid" },
                pools = { health = { maximum = 10 } },
            })
        end
        form("cocoon")
        form("moth")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 10 } },
            morphs = { {
                into = "moth",
                via = { form = "cocoon", interrupted = "dies" },
                time = 20, placement = "reserve", cancel = "committed",
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let larva = registry.entity("larva").expect("larva is registered");
    assert_eq!(
        larva.morphs[0].course(),
        &MorphCourse::via("cocoon", [], ViaInterrupted::Dies)
    );
}

#[test]
fn interrupted_beside_via_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            morphs = { {
                into = "moth",
                via = { form = "cocoon", interrupted = "dies" },
                interrupted = "dies",
                time = 20, placement = "reserve", cancel = "committed",
            } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an interrupted beside a via");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("morph interrupted must be nothing beside a via, which says its own, found 'dies'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_revert_carry_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            morphs = { {
                into = "moth",
                via = { form = "cocoon", interrupted = { reverts = { health = "half" } } },
                time = 20, placement = "reserve", cancel = "committed",
            } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown revert carry");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("revert carry must be 'restore', 'share', 'difference', 'clamp', 'full', or 'initial', found 'half'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn carry_of_undefined_pool_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("larva", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            morphs = { { into = "moth", land_pool_carry = { mana = "share" }, time = 20, placement = "reserve", cancel = "committed" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a carry of an undefined pool");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("pool 'mana' is not defined")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn effect_pool_shift_reads_each_keyword() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = {
                { pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } }, pool_shift = "share" },
                { pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } }, pool_shift = "difference" },
                { pool_maximums = { { entity_stat = "max_energy", op = "flat", value = "10" } }, pool_shift = "clamp" },
                { stats = { { entity_stat = "armor", op = "flat", value = "1" } } },
            },
        })
        define_player_buff("drilled", {
            stack = "ignore",
            entity_modifiers = { {
                pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } },
                pool_shift = "difference",
            } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let hearty = registry.entity_buff_def(registry.entity_buff("hearty").unwrap());
    let pool_shifts: Vec<Option<PoolShift>> = hearty
        .effects
        .iter()
        .map(|effect| match effect {
            EntityEffect::Modifiers(EntityModifiers::PoolMaximums { pool_shift, .. }) => {
                Some(*pool_shift)
            }
            EntityEffect::Modifiers(EntityModifiers::Stats(_)) => None,
            EntityEffect::Disable | EntityEffect::Conceal => unreachable!("only modifiers"),
        })
        .collect();
    assert_eq!(
        pool_shifts,
        vec![
            Some(PoolShift::Share),
            Some(PoolShift::Difference),
            Some(PoolShift::Clamp),
            None
        ]
    );
    let drilled = registry.player_buff_def(registry.player_buff("drilled").unwrap());
    assert_eq!(
        drilled.entity_modifiers,
        vec![EntityModifiers::PoolMaximums {
            modifiers: vec![EntityModifier {
                stat: EntityStatId::MAX_HEALTH,
                op: ModifierOp::FlatAdd,
                magnitude: FixedI64::from_num(10),
            }],
            pool_shift: PoolShift::Difference,
        }]
    );
}

#[test]
fn player_buff_reads_each_set_of_entity_modifiers() {
    let source = r#"
        define_player_buff("drilled", {
            stack = "ignore",
            entity_modifiers = {
                { stats = { { entity_stat = "damage", op = "flat", value = "2" } } },
                {
                    pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } },
                    pool_shift = "difference",
                },
            },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let drilled = registry.player_buff_def(registry.player_buff("drilled").unwrap());
    assert_eq!(
        drilled.entity_modifiers,
        vec![
            EntityModifiers::Stats(vec![EntityModifier {
                stat: EntityStatId::DAMAGE,
                op: ModifierOp::FlatAdd,
                magnitude: FixedI64::from_num(2),
            }]),
            EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(10),
                }],
                pool_shift: PoolShift::Difference,
            },
        ]
    );
}

#[test]
fn unknown_pool_shift_errors() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = { { pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } }, pool_shift = "some" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject an unknown pool shift");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("pool shift must be 'share', 'difference', or 'clamp', found 'some'")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn entity_modifiers_naming_both_kinds_errors() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = { { stats = { { entity_stat = "armor", op = "flat", value = "1" } }, pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } }, pool_shift = "share" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a set naming both kinds");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("entity modifiers name exactly one of stats or pool_maximums")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn entity_modifiers_naming_neither_kind_errors() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = { { pool_shift = "share" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a set naming no kind");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("entity modifiers name exactly one of stats or pool_maximums")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn pool_maximums_without_pool_shift_errors() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = { { pool_maximums = { { entity_stat = "max_health", op = "flat", value = "10" } } } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject pool maximums with no shift");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("field 'pool_shift': error converting Lua nil to String")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn pool_shift_beside_stats_errors() {
    let source = r#"
        define_entity_buff("hearty", {
            lasting = "forever",
            stack = "ignore",
            effects = { { stats = { { entity_stat = "armor", op = "flat", value = "1" } }, pool_shift = "share" } },
        })
    "#;
    let Err(error) = content::load(&engine(), source) else {
        panic!("must reject a shift beside stats");
    };
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("a pool_shift goes beside pool_maximums, not stats")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unknown_morph_interrupted_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", land_pool_carry = { health = "share" }, time = 20, placement = "reserve", cancel = "committed", interrupted = "vanishes" },
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
// ─── Concealment and detection ────────────────────────────────────────────────
//

#[test]
fn type_reads_concealment_and_stands_exposed_unless_declared() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("shade", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            concealment = "concealed",
        })
        define_entity("footman", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    assert_eq!(
        registry.entity("shade").unwrap().concealment,
        Concealment::Concealed
    );
    assert_eq!(
        registry.entity("footman").unwrap().concealment,
        Concealment::Exposed
    );
}

#[test]
fn unknown_concealment_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("shade", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            concealment = "shadowy",
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad concealment");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("concealment must be 'exposed' or 'concealed', found 'shadowy'")),
        "{error:?}"
    );
}

#[test]
fn unknown_field_layer_errors() {
    let source = r#"
        define_field("mist", { layer = "everywhere", decay = "instant" })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad field layer");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("field layer must be 'anywhere' or a layer mask, found 'everywhere'")),
        "{error:?}"
    );
}

#[test]
fn unknown_while_disabled_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("power", { layer = GROUND, decay = "instant" })
        define_entity("pylon", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_sources = {
                { field = "power", radius = 6, growth = "instant", while_constructing = "nothing", while_disabled = "dims" },
            },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad while_disabled");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("while_disabled must be 'full', 'nothing', or a { held = cells } table, found 'dims'")),
        "{error:?}"
    );
}

#[test]
fn parses_buff_effects_lasting_and_interruptions() {
    let source = r#"
        define_resource("gold")
        define_entity_buff("wind_walk", {
            lasting = { ticks = 300 },
            stack = "refresh",
            interrupted_by = { "attack", "cast", "hit" },
            effects = {
                "conceal",
                { stats = { { entity_stat = "speed", op = "percent", value = "0.5" } } },
            },
        })
        define_entity_buff("cloaked", {
            lasting = { upkeep = { cost = { energy = "0.25", resources = { gold = 1 } }, period = 20 } },
            stack = "ignore",
            effects = { "conceal" },
        })
        define_entity_buff("stunned", {
            lasting = "forever",
            stack = "ignore",
            effects = { "disable" },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let def = |name: &str| registry.entity_buff_def(registry.entity_buff(name).expect("defined"));

    assert_eq!(
        *def("wind_walk"),
        EntityBuffDef {
            effects: vec![
                EntityEffect::Conceal,
                EntityEffect::Modifiers(EntityModifiers::Stats(vec![EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_str("0.5").unwrap(),
                }])),
            ],
            lasting: Lasting::For(300),
            stack_rule: StackRule::Refresh,
            interrupted_by: vec![Interruption::Attack, Interruption::Cast, Interruption::Hit],
        }
    );
    assert_eq!(
        *def("cloaked"),
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::Upkeep {
                costs: vec![
                    Cost::Resources(price::from([("gold", 1)])),
                    Cost::Energy(FixedU64::from_str("0.25").unwrap()),
                ],
                period: 20,
            },
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        }
    );
    assert_eq!(
        *def("stunned"),
        EntityBuffDef {
            effects: vec![EntityEffect::Disable],
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        }
    );
}

#[test]
fn unknown_interruption_errors() {
    let source = r#"
        define_entity_buff("wind_walk", {
            lasting = "forever",
            stack = "ignore",
            interrupted_by = { "move" },
        })
    "#;
    let error = content::load(&engine(), source)
        .err()
        .expect("bad interruption");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("interruption must be 'attack', 'cast', or 'hit', found 'move'")),
        "{error:?}"
    );
}

#[test]
fn lasting_names_exactly_one_of_ticks_or_upkeep() {
    for lasting in [
        "{ ticks = 3, upkeep = { cost = { energy = \"1\" }, period = 1 } }",
        "{ }",
    ] {
        let source = format!(
            r#"
            define_entity_buff("cloaked", {{
                lasting = {lasting},
                stack = "ignore",
                effects = {{ "conceal" }},
            }})
            "#
        );
        let error = content::load(&engine(), &source)
            .err()
            .expect("bad lasting");
        assert!(
            matches!(&error, ScriptError::ContentError(m) if m.contains("a lasting table names exactly one of ticks, upkeep, or as_long_as")),
            "{lasting}: {error:?}"
        );
    }
}

#[test]
fn unknown_lasting_errors() {
    let source = r#"
        define_entity_buff("cloaked", {
            lasting = "briefly",
            stack = "ignore",
            effects = { "conceal" },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("bad lasting");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("lasting must be 'forever', a { ticks = ... } table, a { upkeep = ... } table, or a { as_long_as = ... } table, found 'briefly'")),
        "{error:?}"
    );
}

#[test]
fn unknown_entity_effect_errors() {
    let source = r#"
        define_entity_buff("cloaked", {
            lasting = "forever",
            stack = "ignore",
            effects = { "hidden" },
        })
    "#;
    let error = content::load(&engine(), source).err().expect("bad effect");
    assert!(
        matches!(&error, ScriptError::ContentError(m) if m.contains("entity effect must be 'disable', 'conceal', or a { stats = ... } or { pool_maximums = ..., pool_shift = ... } table, found 'hidden'")),
        "{error:?}"
    );
}

#[test]
fn field_effect_reads_concealed() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("veil", { layer = "anywhere", decay = "instant" })
        define_entity("zealot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_effects = { { field = "veil", of = "allied", coverage = "every", inside = { "conceal" } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let veil = registry.field("veil").expect("veil defined");
    assert_eq!(
        registry.entity("zealot").unwrap().field_effects,
        vec![FieldEffect::new(
            veil,
            Affiliation::Allied,
            FieldCoverage::Every,
            vec![EntityEffect::Conceal],
            Vec::new(),
            None
        )]
    );
}

#[test]
fn field_effect_reads_both_sides() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("power", { layer = "anywhere", decay = "instant" })
        define_entity("zealot", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            field_effects = { { field = "power", of = "own", coverage = "every",
                inside = { "conceal" }, outside = { "disable", "conceal" } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let power = registry.field("power").expect("power defined");
    assert_eq!(
        registry.entity("zealot").unwrap().field_effects,
        vec![FieldEffect::new(
            power,
            Affiliation::Own,
            FieldCoverage::Every,
            vec![EntityEffect::Conceal],
            vec![EntityEffect::Disable, EntityEffect::Conceal],
            None
        )]
    );
}

#[test]
fn field_effect_reads_holds_while() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_field("creep", { layer = "anywhere", decay = "instant" })
        define_entity("pit", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
            field_effects = { { field = "creep", of = "anyone", coverage = "every", holds_while = "built",
                outside = { { stats = { { entity_stat = "health_drain", op = "flat", value = "0.2" } } } } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let [effect] = registry.entity("pit").unwrap().field_effects.as_slice() else {
        panic!("the pit declares one field effect");
    };
    assert_eq!(effect.holds_while(), Some(&Requirement::Built));
}

#[test]
fn parses_field_detection_and_watch_detection() {
    let source = r#"
        local GROUND = define_layer("ground")
        local AIR = define_layer("air")
        define_field("true_sight", { layer = "anywhere", decay = "instant", detection = GROUND | AIR })
        define_field("veil", { layer = "anywhere", decay = "instant" })
        define_skill("scan", {
            cooldown = 10,
            caster = "entity",
            target = "position",
            effect = { watch = { radius = 6, duration = 40, detection = AIR } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("content loads");
    let ground = registry.layer("ground").expect("ground defined");
    let air = registry.layer("air").expect("air defined");
    let field = |name: &str| *registry.field_def(registry.field(name).expect("defined"));

    assert_eq!(
        field("true_sight").detection(),
        Detection::Reveals(LayerMask::EMPTY | ground | air)
    );
    assert_eq!(field("true_sight").layer(), FieldLayer::Anywhere);
    assert_eq!(field("veil").detection(), Detection::Blind);

    let scan = registry
        .skill("scan")
        .and_then(|id| registry.skill_def(id))
        .expect("scan defined");
    let SkillCaster::Entity { effect, .. } = &scan.caster else {
        panic!("scan is cast by an entity");
    };
    assert_eq!(
        *effect,
        EntityCastEffect::Watch {
            radius: 6,
            duration: 40,
            detection: Detection::Reveals(LayerMask::EMPTY | air),
        }
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
            stats = {{ harvest_range = 1 }},
            pools = {{ health = {{ maximum = 20 }} }},
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
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2,
            damage = 6, attack_range = 4, attack_period = 7, damage_point = 3,
        },
        pools = { health = { maximum = 40 } },
        dying = { time = 2 },
        attack = { targets = GROUND },
        price = { gold = 80 },
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
        stats = { speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", build_range = 1, harvest_range = 1 },
        pools = { health = { maximum = 30 } },
        dying = { time = 2 },
        price = { gold = 50 },
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
        pools = { health = { maximum = 800 } },
        dying = { time = 2 },
        price = { gold = 400 },
        build = { time = 200, pools = { health = "initial" } },
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
        pools = { health = { maximum = 100 } },
        build = { time = 20, pools = { health = "initial" } },
        field_sources = {
            { field = "creep", radius = 10, growth = { cycle = 9, initial_radius = 1 }, while_constructing = { held = 1 }, while_disabled = "full" },
        },
    })

    define_entity("pylon", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        pools = { health = { maximum = 100 } },
        field_sources = {
            { field = "power", radius = 6, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
        on_stand = {
            { field = { field = "creep", radius = 6, action = "clear" } },
        },
    })

    define_entity("gateway", {
        location = { occupation = GROUND, size = 2, solidity = "solid" },
        pools = { health = { maximum = 100 } },
        field_placement = {
            { requires = "power", of = "own", coverage = "any" },
            { forbids = "creep" },
        },
        field_effects = {
            { field = "power", of = "own", coverage = "every", outside = { "disable" } },
        },
    })

    define_entity("zergling", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1 },
        pools = { health = { maximum = 20 } },
        field_effects = {
            { field = "creep", of = "anyone", coverage = "any", inside = { {
                stats = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } } },
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
        define_entity_stat("morph_time", 1)

        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5", morph_time = 20 },
            pools = { health = { maximum = 40 }, energy = { maximum = 50 } },
            tags = { "winged" },
            morphs = {
                { into = "flier", land_pool_carry = { health = "share" },
                  time = { stat = "morph_time" },
                  placement = "revalidate",
                  cancel = "committed",
                  cost = { energy = "20" },
                  requires = { { tag = "winged" } } },
                { into = "statue", land_pool_carry = { health = "share" },
                  via = { form = "chrysalis", enter_pool_carry = { health = "full" }, interrupted = { reverts = { health = "restore" } } },
                  time = 40,
                  placement = "reserve",
                  cancel = "refundable",
                  cost = { resources = { gold = 30 } } },
            },
        })
        define_entity("flier", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            pools = { health = { maximum = 40 } },
        })
        define_entity("chrysalis", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 60 } },
        })
        define_entity("statue", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            pools = { health = { maximum = 100 } },
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
    assert_eq!(first.time(), Quantity::Stat(morph_time));
    assert_eq!(first.placement(), MorphPlacement::Revalidate);
    assert_eq!(first.cancel(), MorphCancel::Committed);
    assert_eq!(first.costs(), [Cost::Energy(FixedU64::from_num(20))]);
    assert_eq!(first.requires(), [Requirement::Tag("winged".to_string())]);

    assert_eq!(second.into_type(), "statue");
    assert_eq!(second.time(), Quantity::Constant(40));
    assert_eq!(second.placement(), MorphPlacement::Reserve);
    assert_eq!(second.cancel(), MorphCancel::Refundable);
    assert_eq!(
        second.costs(),
        [Cost::Resources(price::from([("gold", 30)]))]
    );
    assert!(
        second.requires().is_empty(),
        "a transition stating no requirements is gated by none"
    );
    assert_eq!(
        first.course(),
        &MorphCourse::direct(MorphInterrupted::Reverts)
    );
    assert_eq!(
        second.course(),
        &MorphCourse::via(
            "chrysalis",
            [(PoolId::HEALTH, PoolCarry::Full)],
            ViaInterrupted::reverts([(PoolId::HEALTH, RevertCarry::Restore)])
        )
    );
}

#[test]
fn unknown_morph_placement_errors() {
    let source = r#"
        local GROUND = define_layer("ground")
        define_entity("walker", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { speed = 1, turn_rate = 30, pivot_rate = 30, radius = "0.5" },
            morphs = {
                { into = "flier", land_pool_carry = { health = "share" }, time = 20, placement = "hover", cancel = "committed" },
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
                { into = "flier", land_pool_carry = { health = "share" }, time = 20, placement = "reserve", cancel = "maybe" },
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
                { into = "flier", land_pool_carry = { health = "share" }, time = "fast", placement = "reserve", cancel = "committed" },
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
                { into = "flier", land_pool_carry = { health = "share" }, time = { stat = "bogus" }, placement = "reserve", cancel = "committed" },
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
            stats = { speed = "0.3", radius = "1", weight = "2",
                turn_rate = 30, pivot_rate = 30, aim_rate = 30,
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            pools = { health = { maximum = 40 } },
            turrets = { { turret = "cannon", at = { 0, 0 }, size = { 2, 2 } } },
        })
    "#;
    let registry = content::load(&engine(), source).expect("load content");

    let cannon = registry.turret("cannon").expect("turret defined");
    assert_eq!(
        registry.turret_def(cannon),
        &TurretDef::new(
            Weapon::new(LayerId::new(1), Delivery::Instant, None, Slain::Remains),
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
        .with_pool(Pool::health(40))
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
        define_entity_stat("flak_damage", 0)

        define_turret("flak", {
            targets = GROUND,
            stats = { damage = "flak_damage" },
        })

        define_entity("keep", {
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = { flak_damage = 3,
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            pools = { health = { maximum = 100 } },
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
                damage = 6, attack_range = 4, acquire_range = 6, attack_period = 7, damage_point = 3,
            },
            pools = { health = { maximum = 40 } },
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
