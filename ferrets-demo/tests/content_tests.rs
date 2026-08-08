//! The demo's embedded content script and map: they load, validate, and agree
//! with each other.

use ferrets_demo::{content::CONTENT, map};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    content::{
        entity_stats::EntityStatId,
        skills::{EntityCastTarget, PlayerCastEffect, SkillCaster},
        work::WorkPresence,
    },
    map::Map,
};

#[test]
fn content_loads_and_validates() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    for name in [
        "gold_mine",
        "tree",
        "peasant",
        "town_hall",
        "farm",
        "barracks",
        "archer",
        "mortar",
        "medic",
        "peon",
        "great_hall",
        "pig_farm",
        "war_camp",
        "grunt",
        "shaman",
        "ship",
        "sea_fortress",
    ] {
        assert!(registry.entity(name).is_some(), "missing entity '{name}'");
    }
    assert!(registry.has_race("human") && registry.has_race("orc"));
    assert!(registry.has_layer(map::GROUND) && registry.has_layer(map::WATER));
    assert!(registry.has_terrain("grass") && registry.has_terrain("water"));
}

#[test]
fn farms_provide_supply_and_units_carry_supply_cost() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    for name in ["farm", "pig_farm"] {
        let farm = registry.entity(name).expect("farm is registered");
        assert!(
            farm.base_stat(EntityStatId::SUPPLY_PROVIDED)
                .is_some_and(|provided| provided > FixedU64::ZERO),
            "'{name}' provides supply, or the race has nothing to raise its cap with"
        );
    }

    for name in ["peasant", "archer", "peon", "grunt", "shaman", "ship"] {
        let unit = registry.entity(name).expect("unit is registered");
        assert!(
            unit.base_stat(EntityStatId::SUPPLY_COST)
                .is_some_and(|cost| cost > FixedU64::ZERO),
            "'{name}' occupies supply, or farms would gate nothing it does"
        );
    }
}

#[test]
fn worker_presences_cover_every_variant_and_differ_by_race() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let presences = |name: &str| {
        let def = registry.entity(name).expect("worker is registered");
        let carrier = def
            .resource_carrier
            .as_ref()
            .expect("workers carry resources");
        vec![
            def.builder.as_ref().expect("workers build").presence(),
            def.repairer.as_ref().expect("workers mend").presence(),
            carrier
                .harvest_data("wood")
                .expect("workers chop")
                .presence(),
            carrier
                .harvest_data("gold")
                .expect("workers mine")
                .presence(),
        ]
    };

    let peasant = presences("peasant");
    let peon = presences("peon");

    // Every variant has to be reachable in play, or one of them can only ever be
    // exercised by the test suite.
    for variant in [
        WorkPresence::Hidden,
        WorkPresence::Present,
        WorkPresence::PresentStacking,
    ] {
        assert!(
            peasant.contains(&variant) || peon.contains(&variant),
            "no demo worker declares {variant:?}, so it cannot be tried in the game"
        );
    }
    assert_ne!(
        peasant, peon,
        "the two races are meant to attend their work differently"
    );
}

#[test]
fn war_drums_rallies_owned_units_for_stockpile_price() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let war_drums = registry
        .skill("war_drums")
        .expect("war_drums is registered");
    let def = registry
        .skill_def(war_drums)
        .expect("handle came from this registry");
    let SkillCaster::Player { cost, effect } = &def.caster else {
        panic!("war_drums is a player cast");
    };
    let PlayerCastEffect::ApplyBuff(buff) = effect else {
        panic!("the rallying call applies a buff");
    };
    let buff = registry.player_buff_def(*buff);
    assert!(
        buff.entity_modifiers
            .iter()
            .any(|modifier| modifier.stat == EntityStatId::SPEED),
        "the rallying call moves the army's speed, or casting it changes nothing visible"
    );
    assert!(
        cost.contains_key("gold"),
        "the cast is paid from the stockpile, or it costs the player nothing"
    );
}

#[test]
fn grunt_and_shaman_carry_their_abilities() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let blood_rite = registry
        .skill("blood_rite")
        .expect("blood_rite is registered");
    let grunt = registry.entity("grunt").expect("grunt is registered");
    assert!(
        grunt.skills.contains(&blood_rite),
        "the grunt carries blood_rite, or the health-cost arm has no demo button"
    );

    let second_wind = registry
        .skill("second_wind")
        .expect("second_wind is registered");
    let shaman = registry.entity("shaman").expect("shaman is registered");
    assert!(
        shaman.skills.contains(&second_wind),
        "the shaman carries second_wind, or the mend has no caster"
    );
    let def = registry
        .skill_def(second_wind)
        .expect("handle came from this registry");
    assert!(
        matches!(
            def.caster,
            SkillCaster::Entity {
                target: EntityCastTarget::Ally,
                ..
            }
        ),
        "second_wind mends a clicked ally, or the shaman can only heal itself"
    );
}

#[test]
fn map_builds_against_content_with_lake_blocking_ground() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let data = map::data();
    let live = Map::from_data(&data, &registry);

    let ground = registry.layer(map::GROUND).unwrap();
    let water = registry.layer(map::WATER).unwrap();

    // The lake center floats ships and blocks walkers; a corner is the inverse.
    let lake = CellPos::new(48, 48);
    assert!(!live.nav_grid().is_passable(ground, lake));
    assert!(live.nav_grid().is_passable(water, lake));

    let corner = CellPos::new(1, 1);
    assert!(live.nav_grid().is_passable(ground, corner));
    assert!(!live.nav_grid().is_passable(water, corner));
}

#[test]
fn boss_placements_sit_on_water() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let data = map::data();
    let live = Map::from_data(&data, &registry);
    let water = registry.layer(map::WATER).unwrap();

    for placement in data
        .placements()
        .iter()
        .filter(|p| p.owner == Some(map::BOSS))
    {
        let size = registry
            .entity(&placement.type_name)
            .and_then(|def| def.location.as_ref())
            .map(|location| location.size())
            .expect("boss placement type has a location");
        for dy in 0..size.height {
            for dx in 0..size.width {
                let cell = CellPos::new(placement.cell.0 + dx, placement.cell.1 + dy);
                assert!(
                    live.nav_grid().is_passable(water, cell),
                    "boss '{}' cell ({}, {}) is not open water",
                    placement.type_name,
                    cell.x,
                    cell.y
                );
            }
        }
    }
}
