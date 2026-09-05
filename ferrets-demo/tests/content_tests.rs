//! The demo's embedded content script and map: they load, validate, and agree
//! with each other.

use ferrets_content::{
    build::BuilderAttendance,
    costs,
    entity_stats::EntityStatId,
    field::{FieldAction, FieldCoverage, FieldPlacement, FieldVision},
    morph::{MorphCancel, MorphPlacement, MorphTime},
    skills::{EntityCastCost, EntityCastTarget, PlayerCastEffect, SkillCaster},
    stand::StandingAct,
    targeting,
    work::WorkPresence,
};
use ferrets_demo::{content::CONTENT, map};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::map::Map;

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
        "gryphon",
        "gryphon_aloft",
        "zeppelin",
        "hive",
        "drone",
        "tumor",
        "brood_nest",
        "spawning_pit",
        "swarmling",
        "cocoon",
        "ravager",
        "nexus",
        "probe",
        "pylon",
        "gateway",
        "photon_cannon",
        "zealot",
    ] {
        assert!(registry.entity(name).is_some(), "missing entity '{name}'");
    }
    assert!(registry.has_race("human") && registry.has_race("orc"));
    assert!(registry.has_race("swarm") && registry.has_race("conclave"));
    assert!(registry.field("creep").is_some() && registry.field("power").is_some());
    assert!(
        registry.has_layer(map::GROUND)
            && registry.has_layer(map::WATER)
            && registry.has_layer(map::AIR)
    );
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
        let BuilderAttendance::Crew(building) =
            def.builder.as_ref().expect("workers build").attendance()
        else {
            panic!("the old races' workers attend their sites");
        };
        vec![
            building,
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

    // The field races' workers do not attend their sites, each in its own way.
    let builds_as = |name: &str| {
        registry
            .entity(name)
            .and_then(|def| def.builder.as_ref())
            .map(|builder| builder.attendance())
            .expect("the worker builds")
    };
    assert_eq!(builds_as("probe"), BuilderAttendance::Unattended);
    assert_eq!(builds_as("drone"), BuilderAttendance::Consumed);
}

#[test]
fn swarm_structures_are_built_by_drone_they_consume() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let drone = registry.entity("drone").expect("drone is registered");
    let builder = drone.builder.as_ref().expect("the drone builds");

    // A drone is consumed by its site, and nothing of the swarm's mends: a hurt
    // structure stays hurt. Structures keep ordinary build terms.
    assert_eq!(builder.attendance(), BuilderAttendance::Consumed);
    for name in [
        "drone",
        "swarmling",
        "hive",
        "tumor",
        "brood_nest",
        "spawning_pit",
    ] {
        let def = registry.entity(name).expect("swarm type is registered");
        assert!(def.repairer.is_none(), "'{name}' must not repair");
    }
    for name in ["hive", "tumor", "brood_nest", "spawning_pit"] {
        assert!(builder.can_build(name), "the drone builds '{name}'");
        let structure = registry
            .entity(name)
            .expect("swarm structure is registered");
        assert!(
            structure.build_time.is_some() && !structure.cost.is_empty(),
            "'{name}' is built and priced like any other structure"
        );
    }
    assert!(
        drone.morphs.is_empty(),
        "a drone changes into nothing; it is consumed"
    );
}

#[test]
fn swarmling_grows_into_ravager_inside_cocoon() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let swarmling = registry
        .entity("swarmling")
        .expect("swarmling is registered");
    let [growth] = swarmling.morphs.as_slice() else {
        panic!("the swarmling has exactly one change of form");
    };

    assert_eq!(growth.into_type(), "ravager");
    assert_eq!(growth.via_type(), Some("cocoon"));
    assert_eq!(growth.cancel(), MorphCancel::Refundable);
    assert!(growth.requires().contains(&"spawning_pit".to_string()));
    assert!(
        growth
            .costs()
            .iter()
            .any(|cost| matches!(cost, EntityCastCost::Resources(_))),
        "growing costs the stockpile"
    );

    // The cocoon is helpless and the ravager is not.
    let cocoon = registry.entity("cocoon").expect("cocoon is registered");
    assert!(!cocoon.can_move() && !cocoon.can_attack());
    let ravager = registry.entity("ravager").expect("ravager is registered");
    assert!(ravager.can_move() && ravager.can_attack());
}

#[test]
fn creep_watches_for_its_spreader_and_power_does_not() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let creep = registry.field("creep").expect("creep field is registered");
    let power = registry.field("power").expect("power field is registered");

    assert_eq!(registry.field_def(creep).vision(), FieldVision::Watched);
    assert_eq!(registry.field_def(power).vision(), FieldVision::Dark);
    // The tumor leaves the watching to its creep.
    let tumor = registry.entity("tumor").expect("tumor is registered");
    assert_eq!(
        tumor.base_stat(EntityStatId::SIGHT_RANGE),
        Some(FixedU64::ONE)
    );
}

#[test]
fn conclave_probe_places_sites_and_nexus_projects_power() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let power = registry.field("power").expect("power field is registered");

    let probe = registry.entity("probe").expect("probe is registered");
    assert_eq!(
        probe.builder.as_ref().map(|builder| builder.attendance()),
        Some(BuilderAttendance::Unattended)
    );
    assert!(probe.repairer.is_none(), "the probe must not repair");

    // Both the nexus and the pylon power the ground around them, so the first
    // gateway needs no pylon before it.
    for name in ["nexus", "pylon"] {
        let def = registry
            .entity(name)
            .expect("conclave structure is registered");
        assert!(
            def.field_sources
                .iter()
                .any(|source| { source.field() == power }),
            "'{name}' projects power"
        );
    }

    // A powered structure needs power under its whole footprint, not just its
    // anchor.
    for name in ["gateway", "photon_cannon"] {
        let def = registry
            .entity(name)
            .expect("conclave structure is registered");
        assert!(
            def.field_placement.iter().any(|rule| matches!(
                rule,
                FieldPlacement::Requires { field, coverage: FieldCoverage::Footprint, .. }
                    if *field == power
            )),
            "'{name}' needs power under its whole footprint"
        );
    }

    // A pylon that comes to stand burns away creep nothing sustains around it.
    let creep = registry.field("creep").expect("creep field is registered");
    let pylon = registry.entity("pylon").expect("pylon is registered");
    assert!(pylon.on_stand.iter().any(|act| matches!(
        act,
        StandingAct::Field { field, action: FieldAction::Clear, .. } if *field == creep
    )));
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

//
// ─── Air layer ─────────────────────────────────────────────────────────────────
//

#[test]
fn fliers_live_on_air_alone() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let air = registry.layer(map::AIR).expect("air layer is registered");

    for flier in ["gryphon_aloft", "zeppelin"] {
        let occupation = registry
            .entity(flier)
            .and_then(|def| def.location.as_ref())
            .map(|location| location.occupation())
            .expect("the flier has a location");

        assert!(
            occupation == *air,
            "'{flier}' occupies {occupation} rather than the air layer alone, so surface \
             occupancy would block it"
        );
    }
}

#[test]
fn air_layer_is_open_where_ground_and_water_are_blocked() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let data = map::data();
    let live = Map::from_data(&data, &registry);

    let ground = registry.layer(map::GROUND).unwrap();
    let water = registry.layer(map::WATER).unwrap();
    let air = registry.layer(map::AIR).unwrap();

    // Open water blocks walkers, dry land blocks ships, and the air is open over
    // both: this is the whole of what the layer buys.
    for cell in [CellPos::new(48, 48), CellPos::new(1, 1)] {
        assert!(
            live.nav_grid().is_passable(air, cell),
            "air is blocked at ({}, {})",
            cell.x,
            cell.y
        );
    }
    assert!(!live.nav_grid().is_passable(ground, CellPos::new(48, 48)));
    assert!(!live.nav_grid().is_passable(water, CellPos::new(1, 1)));
}

#[test]
fn only_tall_fortress_occupies_air() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let air = registry.layer(map::AIR).expect("air layer is registered");

    // Exactly one standing thing is tall enough to be in a flier's way. If more
    // ever are, the air layer stops being mostly open and every flight across
    // the map changes character, which is worth having to say deliberately.
    let tall: Vec<&str> = registry
        .entities()
        .filter(|def| !def.can_move())
        .filter(|def| {
            def.location
                .is_some_and(|location| location.occupation() & air != 0)
        })
        .map(|def| def.name.as_str())
        .collect();

    assert_eq!(tall, ["sea_fortress"]);
}

//
// ─── Targeting layers ──────────────────────────────────────────────────────────
//

#[test]
fn only_melee_and_siege_exclude_air() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let air = registry.layer(map::AIR).expect("air layer is registered");

    // Every weapon declares its layers; what stays deliberate per type is what
    // it leaves out. Only the melee blades, the shell and the wagon's flat gun
    // cannot answer what flies.
    let grounded: Vec<&str> = registry
        .entities()
        .filter(|def| def.can_attack())
        .filter(|def| registry.targets_of(def) & air == 0)
        .map(|def| def.name.as_str())
        .collect();

    assert_eq!(
        grounded,
        [
            "grunt",
            "mortar",
            "ravager",
            "swarmling",
            "war_wagon",
            "zealot"
        ]
    );
}

#[test]
fn narrowed_weapons_still_reach_lake_boss() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // The boss and its fleet live on the water layer, so a weapon narrowed with a
    // whitelist of ground and air would silently stop being able to fight them.
    // This is the regression that made "narrow by exclusion" the rule.
    for attacker in ["grunt", "mortar"] {
        let attacker = registry.entity(attacker).expect("attacker is registered");
        for victim in ["ship", "sea_fortress"] {
            let victim = registry.entity(victim).expect("victim is registered");
            assert!(
                targeting::reaches(registry.targets_of(attacker), victim),
                "'{}' can no longer reach '{}', so the lake boss is unfightable",
                attacker.name,
                victim.name
            );
        }
    }
}

#[test]
fn melee_cannot_reach_flier_but_ranged_can() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let flier = registry.entity("gryphon_aloft").expect("flier");

    assert!(
        !targeting::reaches(
            registry.targets_of(registry.entity("grunt").expect("grunt")),
            flier
        ),
        "melee reaches the air, so taking off would never shake a pursuer"
    );
    assert!(
        !targeting::reaches(
            registry.targets_of(registry.entity("mortar").expect("mortar")),
            flier
        ),
        "siege reaches the air, so it would shell a flier its blast cannot touch"
    );
    // The shaman is deliberately absent: it carries skills, not a weapon, so
    // it reaches nothing — the old reach-everything default used to hide that.
    // The watch tower is absent for the same reason: it only watches, and the
    // bolts belong to its upgrade.
    for answer in ["archer", "ship", "guard_tower"] {
        let answer = registry.entity(answer).expect("anti-air is registered");
        assert!(
            targeting::reaches(registry.targets_of(answer), flier),
            "'{}' cannot reach a flier, so nothing on that side answers one",
            answer.name
        );
    }
}

#[test]
fn watch_tower_stands_on_ground_and_answers_to_anti_air() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let ground = registry.layer(map::GROUND).unwrap();
    let air = registry.layer(map::AIR).unwrap();

    let tower = registry.entity("watch_tower").expect("tower is registered");
    let occupation = tower
        .location
        .map(|location| location.occupation())
        .expect("the tower has a location");

    // It holds the ground alone, so fliers pass over it...
    assert_eq!(occupation, *ground);
    // ...yet it is answerable in the air, which occupation could not have said.
    assert_eq!(targeting::targetable(tower), ground | air);
}

#[test]
fn watch_tower_watches_and_guard_tower_fights() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let watch = registry.entity("watch_tower").expect("tower is registered");
    let guard = registry
        .entity("guard_tower")
        .expect("upgraded tower is registered");

    // The watcher is unarmed; the bolts are what the upgrade buys.
    assert!(
        watch.base_stat(EntityStatId::DAMAGE).is_none(),
        "the watch tower carries a weapon, so the upgrade buys nothing"
    );
    assert!(
        guard.base_stat(EntityStatId::DAMAGE).is_some(),
        "the guard tower is unarmed, so nothing on the orc side answers fliers"
    );
    // And the upgrade trades eyes for those bolts: the watcher sees farther
    // than the fighter it becomes.
    assert_eq!(
        watch.base_stat(EntityStatId::SIGHT_RANGE),
        Some(FixedU64::from_num(12))
    );
    assert_eq!(
        guard.base_stat(EntityStatId::SIGHT_RANGE),
        Some(FixedU64::from_num(10)),
        "the watch tower must outsee its armed upgrade"
    );
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

#[test]
fn gryphon_edges_wear_different_terms() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // Taking off checks nothing early and costs a beat of energy, read from the
    // form's own quickenable stat; landing is free, paced by a plain tick
    // count, and reserves its ground. Both are committed.
    let grounded = registry.entity("gryphon").expect("gryphon is registered");
    let [take_off] = grounded.morphs.as_slice() else {
        panic!("the grounded gryphon declares exactly one transition");
    };
    let morph_time = registry
        .entity_stat("morph_time")
        .expect("demo declares its morph_time stat");
    assert_eq!(take_off.into_type(), "gryphon_aloft");
    assert_eq!(
        take_off.time(),
        MorphTime::Stat(morph_time),
        "the take-off window is not the quickenable stat"
    );
    assert_eq!(take_off.placement(), MorphPlacement::Revalidate);
    assert_eq!(take_off.cancel(), MorphCancel::Committed);
    assert_eq!(
        take_off.costs(),
        [EntityCastCost::Energy(FixedU64::from_num(20))]
    );

    let aloft = registry
        .entity("gryphon_aloft")
        .expect("airborne form is registered");
    let [landing] = aloft.morphs.as_slice() else {
        panic!("the airborne gryphon declares exactly one transition");
    };
    assert_eq!(landing.into_type(), "gryphon");
    assert_eq!(landing.time(), MorphTime::Constant(20));
    assert_eq!(landing.placement(), MorphPlacement::Reserve);
    assert_eq!(landing.cancel(), MorphCancel::Committed);
    assert!(landing.costs().is_empty(), "landing is free");
    assert!(
        aloft.train_time.is_none() && aloft.cost.is_empty(),
        "the airborne form is not producible, only changeable into"
    );
}

#[test]
fn tower_upgrade_is_paid_and_refundable() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let tower = registry.entity("watch_tower").expect("tower is registered");
    let [upgrade] = tower.morphs.as_slice() else {
        panic!("the watch tower declares exactly one transition");
    };
    assert_eq!(upgrade.into_type(), "guard_tower");
    assert_eq!(upgrade.time(), MorphTime::Constant(60));
    assert_eq!(upgrade.placement(), MorphPlacement::Reserve);
    assert_eq!(upgrade.cancel(), MorphCancel::Refundable);
    assert_eq!(
        upgrade.costs(),
        [EntityCastCost::Resources(costs::cost([
            ("gold", 80),
            ("wood", 20)
        ]))]
    );

    let upgraded = registry
        .entity("guard_tower")
        .expect("upgraded tower is registered");
    assert!(
        upgraded.train_time.is_none() && upgraded.build_time.is_none(),
        "the upgraded tower is not producible, only changeable into"
    );
    assert!(
        upgraded.tags.contains("building"),
        "an upgraded tower must still count as a standing base"
    );
}
