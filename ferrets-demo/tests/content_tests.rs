//! The demo's embedded content script and map: they load, validate, and agree
//! with each other.

use ferrets_content::{
    affiliation::Affiliation,
    attack::Slain,
    brood::{BroodlingDef, Lingering, OrphanFate},
    build::BuilderAttendance,
    costs,
    dying::DeathKind,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{FieldAction, FieldCoverage, FieldEffectKind, FieldPlacement, FieldSide, FieldVision},
    kinds::{Kind, Kinds},
    location::Solidity,
    morph::{MorphCancel, MorphInterrupted, MorphPlacement, MorphReason},
    quantity::Quantity,
    registry::ContentRegistry,
    requirement::Requirement,
    resource::Banking,
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach, SkillCaster,
    },
    stand::StandingAct,
    targeting,
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_demo::{content::CONTENT, map};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
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
        "training_camp",
        "archer",
        "mortar",
        "medic",
        "peon",
        "great_hall",
        "big_rock",
        "pig_farm",
        "war_camp",
        "grunt",
        "shaman",
        "ship",
        "sea_fortress",
        "gryphon",
        "gryphon_aloft",
        "zeppelin",
        "hatchery",
        "hive_cocoon",
        "hive",
        "larva",
        "egg",
        "drone",
        "tumor",
        "overlord",
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
        "wisp",
        "huntress",
        "tree_of_life",
        "tree_of_life_uprooted",
        "moon_well",
        "ancient_of_war",
        "ancient_of_war_uprooted",
        "ancient_protector",
        "ancient_protector_uprooted",
        "entangled_mine",
        "scv",
        "command_center",
        "command_center_aloft",
        "barracks",
        "barracks_aloft",
        "factory",
        "factory_aloft",
        "comsat_station",
        "tech_lab",
        "supply_depot",
        "refinery",
        "marine",
        "tank",
        "siege_tank",
    ] {
        assert!(registry.entity(name).is_some(), "missing entity '{name}'");
    }
    assert!(registry.has_race("human") && registry.has_race("orc"));
    assert!(registry.has_race("swarm") && registry.has_race("conclave"));
    assert!(registry.has_race("elves") && registry.has_race("terran"));
    assert!(registry.field("creep").is_some() && registry.field("power").is_some());
    assert!(registry.research("siege_tech").is_some());
    assert!(registry.skill("scanner_sweep").is_some());
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

    for name in ["farm", "pig_farm", "supply_depot"] {
        let farm = registry.entity(name).expect("farm is registered");
        assert!(
            farm.base_stat(EntityStatId::SUPPLY_PROVIDED)
                .is_some_and(|provided| provided > FixedU64::ZERO),
            "'{name}' provides supply, or the race has nothing to raise its cap with"
        );
    }

    for name in [
        "peasant", "archer", "peon", "grunt", "shaman", "ship", "scv", "marine", "tank",
    ] {
        let unit = registry.entity(name).expect("unit is registered");
        assert!(
            unit.base_stat(EntityStatId::SUPPLY_COST)
                .is_some_and(|cost| cost > FixedU64::ZERO),
            "'{name}' occupies supply, or farms would gate nothing it does"
        );
    }
}

#[test]
fn worker_presences_are_all_reachable_in_play_and_differ_by_race() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let presences = |name: &str| {
        let def = registry.entity(name).expect("worker is registered");
        let carrier = def
            .resource_carrier
            .as_ref()
            .expect("workers carry resources");
        let BuilderAttendance::Crew(building) = def
            .builder
            .as_ref()
            .expect("workers build")
            .attendance()
            .clone()
        else {
            panic!("the old races' workers attend their sites");
        };
        vec![
            building,
            def.repairer
                .as_ref()
                .expect("workers mend")
                .presence()
                .clone(),
            carrier
                .harvest_data("wood")
                .expect("workers chop")
                .presence()
                .clone(),
            carrier
                .harvest_data("gold")
                .expect("workers mine")
                .presence()
                .clone(),
        ]
    };

    let peasant = presences("peasant");
    let peon = presences("peon");

    // Each of these has to be reachable in play, or it can only ever be
    // exercised by the test suite. They are the presences demo content
    // declares, not the whole space `CrewLimit` opened up: no demo worker
    // names a finite crew above one.
    for variant in [
        WorkPresence::Hidden {
            crew: CrewLimit::ONE,
        },
        WorkPresence::Present {
            crew: CrewLimit::ONE,
        },
        WorkPresence::Present {
            crew: CrewLimit::Unlimited,
        },
    ] {
        assert!(
            peasant.contains(&variant) || peon.contains(&variant),
            "no demo worker declares {variant:?}, so it cannot be tried in the game"
        );
    }
    assert!(
        peon.iter()
            .any(|presence| matches!(presence, WorkPresence::Attached(_))),
        "the peon attaches to its site"
    );
    assert_ne!(
        peasant, peon,
        "the two races are meant to attend their work differently"
    );

    // The field races' workers do not attend their sites, each in its own way.
    let builds_as = |name: &str| {
        registry
            .entity(name)
            .and_then(|def| def.builder.as_ref())
            .map(|builder| builder.attendance().clone())
            .expect("the worker builds")
    };
    assert_eq!(builds_as("probe"), BuilderAttendance::Unattended);
    assert_eq!(builds_as("drone"), BuilderAttendance::Consumed);
}

#[test]
fn wisp_banks_where_it_sits_and_elves_store_nothing() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let wisp = registry.entity("wisp").expect("wisp is registered");
    let carrier = wisp.resource_carrier.as_ref().expect("the wisp carries");

    // Alone in a tree it never fells, with any number of others on a mine it
    // drains — and the take goes straight to the stockpile either way.
    let wood = carrier.harvest_data("wood").expect("the wisp draws wood");
    assert_eq!(
        *wood.presence(),
        WorkPresence::Attached(Attachment::new(
            "canopy",
            BerthStance::Orbit {
                radius: FixedU64::lit("0.3"),
                period: 60,
            },
        ))
    );
    assert_eq!(wood.banking(), Banking::Direct);
    assert_eq!(wood.drain(), 0);
    let gold = carrier.harvest_data("gold").expect("the wisp draws gold");
    assert_eq!(
        *gold.presence(),
        WorkPresence::Attached(Attachment::new(
            "rim",
            BerthStance::Roaming {
                speed: FixedU64::lit("0.05"),
                dwell: 40,
            },
        ))
    );
    assert_eq!(gold.banking(), Banking::Direct);
    assert_eq!(gold.drain(), gold.capacity());

    // The tree seats one wisp in its canopy; a plain gold mine seats nobody,
    // so gold takes the entangled mine, raised over it: twelve spots round the
    // rim, three a side every half cell, and five wisps at a time on them.
    let tree = registry.entity("tree").expect("tree is registered");
    let canopy = tree.berths.as_ref().unwrap().group("canopy").unwrap();
    assert_eq!((canopy.points().len(), canopy.slots()), (1, 1));
    assert!(registry.entity("gold_mine").unwrap().berths.is_none());
    let entangled = registry
        .entity("entangled_mine")
        .expect("entangled mine is registered");
    assert_eq!(entangled.overbuilds.as_deref(), Some("gold_mine"));
    let rim = entangled.berths.as_ref().unwrap().group("rim").unwrap();
    assert_eq!((rim.points().len(), rim.slots()), (12, 5));
    // Loop order from the north-west, a fifth of a cell in from the edge.
    assert_eq!(
        rim.points()[..3].to_vec(),
        vec![
            berth("0.5", "0.2"),
            berth("1.0", "0.2"),
            berth("1.5", "0.2"),
        ]
    );
    assert_eq!(rim.points()[3], berth("1.8", "0.5"));
    assert!(entangled.resource_source.is_some() && entangled.resource_storage.is_none());

    // So nothing of the elves' stores anything, and the wisp is spent on what
    // it builds.
    for name in [
        "tree_of_life",
        "moon_well",
        "ancient_of_war",
        "ancient_protector",
        "entangled_mine",
    ] {
        let def = registry.entity(name).expect("elf structure is registered");
        assert!(def.resource_storage.is_none(), "'{name}' must not store");
    }
    assert_eq!(
        *wisp.builder.as_ref().expect("the wisp builds").attendance(),
        BuilderAttendance::Consumed
    );
}

#[test]
fn elf_structures_root_and_uproot_as_pairs() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // The moon well is the one that stays dug in.
    let well = registry
        .entity("moon_well")
        .expect("moon well is registered");
    assert!(well.morphs.is_empty() && !well.can_move());
    assert!(registry.entity("moon_well_uprooted").is_none());

    for name in ["tree_of_life", "ancient_of_war", "ancient_protector"] {
        let rooted = registry.entity(name).expect("rooted form is registered");
        let uprooted_name = format!("{name}_uprooted");
        let uprooted = registry
            .entity(&uprooted_name)
            .expect("uprooted form is registered");

        // Each names the other; uprooting checks nothing, rooting reserves.
        let [uproot] = rooted.morphs.as_slice() else {
            panic!("'{name}' has exactly one change of form");
        };
        assert_eq!(uproot.into_type(), uprooted_name);
        assert_eq!(uproot.placement(), MorphPlacement::Revalidate);
        let [root] = uprooted.morphs.as_slice() else {
            panic!("'{uprooted_name}' has exactly one change of form");
        };
        assert_eq!(root.into_type(), name);
        assert_eq!(root.placement(), MorphPlacement::Reserve);

        // Same footprint both ways; only the walker moves and bites, only
        // the rooted form produces, and both count as buildings.
        assert_eq!(
            rooted.location.unwrap().size(),
            uprooted.location.unwrap().size()
        );
        assert!(!rooted.can_move() && uprooted.can_move());
        assert!(uprooted.attack.is_some(), "'{uprooted_name}' bites");
        assert!(
            uprooted.trainer.is_none() && uprooted.researcher.is_none(),
            "'{uprooted_name}' produces nothing"
        );
        assert!(rooted.tags.contains("building") && uprooted.tags.contains("building"));
        assert_eq!(
            rooted.base_stat(EntityStatId::SUPPLY_PROVIDED),
            uprooted.base_stat(EntityStatId::SUPPLY_PROVIDED),
            "'{name}' feeds the army walking or rooted"
        );
        // Nothing of the elves' takes root on another race's ground.
        assert!(
            forbids(&registry, rooted, "creep") && forbids(&registry, rooted, "blight"),
            "'{name}' takes root on creep or on blight"
        );
    }

    // The moon well never moves, so its own rule is the only one it has; the
    // entangled mine is the one elf structure raised on creep, over a mine the
    // swarm has covered.
    let moon_well = registry
        .entity("moon_well")
        .expect("moon well is registered");
    assert!(
        moon_well.morphs.is_empty()
            && forbids(&registry, moon_well, "creep")
            && forbids(&registry, moon_well, "blight")
    );
    let entangled = registry
        .entity("entangled_mine")
        .expect("entangled mine is registered");
    assert!(!forbids(&registry, entangled, "creep") && !forbids(&registry, entangled, "blight"));
}

#[test]
fn swarm_structures_are_built_by_drone_they_consume() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let drone = registry.entity("drone").expect("drone is registered");
    let builder = drone.builder.as_ref().expect("the drone builds");

    // A drone is consumed by its site, and nothing of the swarm's mends: a hurt
    // structure stays hurt. Structures keep ordinary build terms.
    assert_eq!(*builder.attendance(), BuilderAttendance::Consumed);
    for name in [
        "drone",
        "swarmling",
        "overlord",
        "hatchery",
        "hive_cocoon",
        "hive",
        "tumor",
        "spawning_pit",
    ] {
        let def = registry.entity(name).expect("swarm type is registered");
        assert!(def.repairer.is_none(), "'{name}' must not repair");
    }
    for name in ["hatchery", "tumor", "spawning_pit"] {
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
fn hatchery_breeds_larvae_that_grow_into_drones_swarmlings_and_overlords() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let hatchery = registry.entity("hatchery").expect("hatchery is registered");
    let brood = hatchery.breeder.as_ref().expect("the hatchery breeds");
    assert_eq!(brood.breeds(), "larva");
    assert_eq!(brood.period(), Quantity::Constant(220));
    assert_eq!(brood.limit(), 3);
    assert_eq!(brood.initial(), 1);
    assert_eq!(
        brood.orphans(),
        OrphanFate::Linger(Lingering::Reseat { distance: 4 })
    );
    assert!(
        hatchery.trainer.is_none(),
        "the hatchery trains nothing: its larvae are its production"
    );
    // Five points along the hall's southern foot, one row outside the
    // footprint, four of them seated at once.
    let group = hatchery
        .berths
        .as_ref()
        .and_then(|berths| berths.group("brood"))
        .expect("the hatchery offers the brood group");
    assert_eq!(group.points().len(), 5);
    assert_eq!(group.slots(), 4);
    assert!(
        group
            .points()
            .iter()
            .all(|point| point.y == FixedI64::lit("3.5"))
    );

    let larva = registry.entity("larva").expect("larva is registered");
    assert_eq!(
        larva.location.map(|location| location.solidity()),
        Some(Solidity::Passable)
    );
    assert!(!larva.can_move(), "a larva is never ordered about");
    // The overlord is the swarm's headroom: a flying two-by-two that needs no
    // creep, so it takes no supply and withers nowhere.
    let overlord = registry.entity("overlord").expect("overlord is registered");
    assert_eq!(
        overlord.location.map(|location| location.size()),
        Some(CellSize::new(2, 2))
    );
    assert!(overlord.can_move(), "an overlord flies");
    assert_eq!(
        overlord.base_stat(EntityStatId::SUPPLY_PROVIDED),
        Some(FixedU64::from_num(6))
    );
    assert!(overlord.base_stat(EntityStatId::SUPPLY_COST).is_none());
    assert!(overlord.field_effects.is_empty());
    let attachment = larva
        .broodling
        .as_ref()
        .map(BroodlingDef::attachment)
        .expect("the larva sits in its hall's berths");
    assert_eq!(attachment.berths(), "brood");
    // Off creep a larva loses its whole pool within a second (20 ticks): the
    // drain has a base to fold into, and the fold empties the pool.
    assert!(larva.base_stat(EntityStatId::HEALTH_DRAIN).is_some());
    let max_health = larva
        .base_stat(EntityStatId::MAX_HEALTH)
        .expect("a larva has health");
    let drain = larva
        .field_effects
        .iter()
        .find(|effect| effect.side() == FieldSide::Outside)
        .and_then(|effect| match effect.kind() {
            FieldEffectKind::Modifiers(modifiers) => modifiers
                .iter()
                .find(|modifier| modifier.stat == EntityStatId::HEALTH_DRAIN)
                .map(|modifier| modifier.magnitude),
            FieldEffectKind::Disabled => None,
        })
        .expect("a larva drains off creep");
    // 1.25 a tick over the 20 ticks of a second is the whole pool of 25.
    assert_eq!(drain, FixedI64::lit("1.25"));
    assert_eq!(max_health, FixedU64::from_num(25));

    let [drone, swarmling, overlord] = larva.morphs.as_slice() else {
        panic!("the larva grows into exactly three things");
    };
    assert_eq!(drone.into_type(), "drone");
    assert_eq!(drone.via_type(), Some("egg"));
    assert_eq!(drone.placement(), MorphPlacement::Nearby);
    assert_eq!(drone.interrupted(), MorphInterrupted::Reverts);
    assert_eq!(drone.reason(), MorphReason::Production);
    assert_eq!(swarmling.into_type(), "swarmling");
    assert!(
        swarmling
            .requires()
            .contains(&Requirement::EntityType("spawning_pit".to_string()))
    );
    // Every growth passes through the egg and counts as production.
    assert_eq!(overlord.into_type(), "overlord");
    assert!(larva.morphs.iter().all(|transition| {
        transition.via_type() == Some("egg") && transition.reason() == MorphReason::Production
    }));
    let pit = registry
        .entity("spawning_pit")
        .expect("spawning_pit is registered");
    assert!(pit.trainer.is_none(), "the pit only unlocks the swarmling");

    // The egg stands solid on the ground it is laid on.
    let egg = registry.entity("egg").expect("egg is registered");
    assert_eq!(
        egg.location.map(|location| location.solidity()),
        Some(Solidity::Solid)
    );
    assert!(!egg.can_move() && !egg.can_attack());
}

#[test]
fn hatchery_grows_into_hive_inside_cocoon_that_carries_its_brood() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let hatchery = registry.entity("hatchery").expect("hatchery is registered");
    let [growth] = hatchery.morphs.as_slice() else {
        panic!("the hatchery grows into exactly one thing");
    };
    assert_eq!(growth.into_type(), "hive");
    assert_eq!(growth.via_type(), Some("hive_cocoon"));
    assert_eq!(growth.reason(), MorphReason::Change);
    assert!(
        growth
            .requires()
            .contains(&Requirement::EntityType("spawning_pit".to_string()))
    );

    // The cocoon offers the same brood berths, so the larvae ride across in
    // them, on creep it keeps spreading; it breeds none on the way.
    let cocoon = registry
        .entity("hive_cocoon")
        .expect("hive_cocoon is registered");
    assert!(
        cocoon
            .berths
            .as_ref()
            .and_then(|berths| berths.group("brood"))
            .is_some()
    );
    assert!(cocoon.breeder.is_none());
    assert!(!cocoon.field_sources.is_empty());

    let hive = registry.entity("hive").expect("hive is registered");
    let brood = hive.breeder.as_ref().expect("the hive breeds");
    assert_eq!(brood.period(), Quantity::Constant(180));
    assert_eq!(brood.initial(), 2);
    assert_eq!(
        brood.orphans(),
        OrphanFate::Linger(Lingering::Reseat { distance: 4 })
    );
    assert!(
        hive.morphs.is_empty() && !produced(&registry, "hive"),
        "the hive is only ever grown from a hatchery"
    );
    // Nothing raises a hive, so it carries what a hatchery cost plus the
    // growth's own price, over both spans: 400 gold and 200 ticks raising the
    // hatchery, 150 gold, 100 wood and 200 ticks growing out of it.
    assert_eq!(hive.cost, costs::cost([("gold", 550), ("wood", 100)]));
    assert_eq!(hive.build_time, Some(400));
    let drone = registry.entity("drone").expect("drone is registered");
    let builder = drone.builder.as_ref().expect("the drone builds");
    assert!(!builder.can_build("hive"));

    // Every swarm structure but the halls stands on creep; the pit withers
    // off it, the tumor spreads its own, and a hall spreads the creep and
    // needs none under it.
    for name in ["tumor", "spawning_pit"] {
        let def = registry
            .entity(name)
            .expect("swarm structure is registered");
        assert!(!def.field_placement.is_empty(), "'{name}' stands on creep");
    }
    let pit = registry
        .entity("spawning_pit")
        .expect("spawning_pit is registered");
    assert!(!pit.field_effects.is_empty(), "the pit withers off creep");
    for name in ["hatchery", "hive_cocoon", "hive"] {
        let def = registry.entity(name).expect("swarm hall is registered");
        assert!(def.field_placement.is_empty(), "'{name}' needs no creep");
        assert!(!def.field_sources.is_empty(), "'{name}' spreads creep");
    }
}

#[test]
fn killed_swarm_structures_burst_into_hatchlings_that_expire() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // Spawn nobody pays for and nobody keeps: four hundred ticks at twenty a
    // second is twenty seconds of a unit, and then it is gone.
    let hatchling = registry
        .entity("hatchling")
        .expect("hatchling is registered");
    assert_eq!(
        hatchling.base_stat_as_u32(EntityStatId::LIFETIME),
        Some(400)
    );
    assert!(
        hatchling.cost.is_empty() && hatchling.train_time.is_none(),
        "spawn is left by a death, never bought"
    );
    assert_eq!(
        hatchling.base_stat_as_u32(EntityStatId::SUPPLY_COST),
        None,
        "spawn feeds off nothing"
    );
    assert!(
        hatchling
            .dying
            .as_ref()
            .expect("spawn takes a moment to fall")
            .leaves()
            .is_empty(),
        "spawn leaves no body of its own"
    );

    // Every structure with life growing inside it spills two, and only when
    // something kills it.
    for name in ["hatchery", "hive_cocoon", "hive", "spawning_pit"] {
        let def = registry
            .entity(name)
            .expect("swarm structure is registered");
        let [burst] = def
            .dying
            .as_ref()
            .expect("swarm structure takes a moment to fall")
            .leaves()
        else {
            panic!("'{name}' hands on exactly one kind of spawn");
        };
        assert_eq!(burst.entity_type(), "hatchling");
        assert_eq!(burst.count(), 2);
        assert!(
            burst.left_by(DeathKind::Killed)
                && !burst.left_by(DeathKind::Cancelled)
                && !burst.left_by(DeathKind::Decayed),
            "'{name}' spills only what is killed out of it"
        );
    }
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
    assert!(
        growth
            .requires()
            .contains(&Requirement::EntityType("hive".to_string()))
    );
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
        probe
            .builder
            .as_ref()
            .map(|builder| builder.attendance().clone()),
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
            &def.caster,
            SkillCaster::Entity {
                target: EntityCastTarget::Standing {
                    side: Affiliation::Allied,
                    ..
                },
                reach: Reach::Wherever,
                ..
            }
        ),
        "second_wind mends a clicked ally, or the shaman can only heal itself"
    );
    let SkillCaster::Entity {
        target: EntityCastTarget::Standing { kinds, .. },
        ..
    } = &def.caster
    else {
        panic!("second_wind aims at something standing");
    };
    // It mends the living and refuses the rest: a shaman cannot patch a wall.
    assert_eq!(kinds, &Kinds::tags(["biological"]));
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

    for flier in [
        "gryphon_aloft",
        "zeppelin",
        "command_center_aloft",
        "barracks_aloft",
        "factory_aloft",
    ] {
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
    // it leaves out. Only the melee blades and bites, the shells, the flat
    // guns of the wagon and the tank, and the undead's flat gun and raised
    // blades cannot answer what flies. The swarm's spawn bites like the
    // swarmling it is too young to be.
    let grounded: Vec<&str> = registry
        .entities()
        .filter(|def| def.can_attack())
        .filter(|def| registry.targets_of(def) & air == 0)
        .map(|def| def.name.as_str())
        .collect();

    assert_eq!(
        grounded,
        [
            "ancient_of_war_uprooted",
            "ancient_protector_uprooted",
            "ghoul",
            "grunt",
            "hatchling",
            "huntress",
            "mortar",
            "necromancer",
            "nerubian_tower",
            "ravager",
            "siege_tank",
            "skeleton",
            "swarmling",
            "tank",
            "tree_of_life_uprooted",
            "war_wagon",
            "zealot"
        ]
    );
}

#[test]
fn shells_deny_bodies_that_rolling_guns_leave() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // Bringing siege is how an army denies a necromancer its material — and
    // it is the shells that do it, planted or carried. Everything that kills
    // by rolling up and firing leaves what it kills where it falls.
    for shelling in ["mortar", "siege_tank"] {
        let def = registry.entity(shelling).expect("siege is registered");
        assert_eq!(
            def.attack.as_ref().expect("siege fights").weapon().slain(),
            Slain::Nothing,
            "'{shelling}' leaves bodies to raise"
        );
    }
    for firing in ["tank", "grunt", "archer"] {
        let def = registry.entity(firing).expect("fighter is registered");
        assert_eq!(
            def.attack
                .as_ref()
                .expect("fighter fights")
                .weapon()
                .slain(),
            Slain::Remains,
            "'{firing}' denies the dead"
        );
    }
}

#[test]
fn machines_are_mended_by_workers_and_sheltered_like_soldiers() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let mortar = registry.entity("mortar").expect("mortar is registered");

    // Every worker mends what is built and what is machined, so a race's
    // siege is never a unit nothing can put back together.
    for name in ["peasant", "peon", "scv"] {
        let worker = registry.entity(name).expect("worker is registered");
        assert!(
            worker
                .repairer
                .as_ref()
                .expect("a worker mends")
                .repairs()
                .admits(mortar),
            "'{name}' cannot mend a machine"
        );
    }
    // And the tube rides where the soldiers ride, which is what its two
    // shelter slots are for.
    let bunker = registry.entity("bunker").expect("bunker is registered");
    assert!(
        bunker
            .transporter
            .as_ref()
            .expect("the bunker shelters")
            .carries()
            .admits(mortar)
    );
    let wagon = registry
        .entity("war_wagon")
        .expect("war wagon is registered");
    assert!(
        wagon.tags.contains("mechanical"),
        "the orc's siege is a machine like every other, or nothing can mend it"
    );
}

#[test]
fn mortar_is_mended_not_healed_and_leaves_no_body() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let mortar = registry.entity("mortar").expect("mortar is registered");

    // A tube on a carriage: the medic passes it by, the SCV patches it up,
    // and what it leaves when it falls is wreckage nobody raises anything
    // from — unlike every other footsoldier of the demo.
    assert!(mortar.tags.contains("mechanical") && !mortar.tags.contains("biological"));
    assert!(
        mortar
            .dying
            .as_ref()
            .expect("the mortar takes a moment to fall")
            .leaves()
            .is_empty()
    );
    let archer = registry.entity("archer").expect("archer is registered");
    assert!(
        archer.tags.contains("biological")
            && !archer
                .dying
                .as_ref()
                .expect("the archer takes a moment to fall")
                .leaves()
                .is_empty(),
        "the infantry it walks with still falls as a body"
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
        Quantity::Stat(morph_time),
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
    assert_eq!(landing.time(), Quantity::Constant(20));
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
    assert_eq!(upgrade.time(), Quantity::Constant(60));
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
        !produced(&registry, "guard_tower"),
        "the upgraded tower is not producible, only changeable into"
    );
    // Priced and paced by what it took to have one standing: 120 gold, 40 wood
    // and 70 ticks raising the watch tower, 80 gold, 20 wood and 60 ticks
    // upgrading it.
    assert_eq!(
        upgraded.cost,
        costs::cost([("gold", 200), ("wood", 60)]),
        "a peon mends the upgraded tower against what it cost to have"
    );
    assert_eq!(upgraded.build_time, Some(130));
    assert!(
        upgraded.tags.contains("building"),
        "an upgraded tower must still count as a standing base"
    );
}

//
// ─── Terrans ──────────────────────────────────────────────────────────────────
//

#[test]
fn terran_gold_comes_through_refinery_alone() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let scv = registry.entity("scv").expect("scv defined");
    let gold = scv
        .resource_carrier
        .as_ref()
        .expect("an scv carries")
        .harvest_data("gold")
        .expect("it carries gold");

    // One scv at a time, inside the building while it works.
    assert_eq!(
        gold.presence(),
        &WorkPresence::Hidden {
            crew: CrewLimit::ONE
        }
    );
    // And a refinery is the only source it may work: a bare seam is one the
    // scv turns down, however much gold is in it.
    assert_eq!(gold.sources(), &Kinds::types(["refinery"]));
    let source = |name: &str| registry.entity(name).expect("source is registered");
    assert!(gold.sources().admits(source("refinery")));
    assert!(!gold.sources().admits(source("gold_mine")));

    // The refinery seats builders and nobody else.
    let refinery = registry.entity("refinery").expect("refinery defined");
    let berths = refinery.berths.as_ref().expect("the refinery seats");
    assert_eq!(berths.group("rim").map(|group| group.slots()), Some(1));
    assert!(berths.group("shaft").is_none());
}

#[test]
fn every_terran_production_building_flies_but_depot() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    for (grounded, aloft) in [
        ("command_center", "command_center_aloft"),
        ("barracks", "barracks_aloft"),
        ("factory", "factory_aloft"),
    ] {
        let down = registry.entity(grounded).expect("grounded form defined");
        let up = registry.entity(aloft).expect("aloft form defined");
        assert_eq!(
            down.morphs.first().map(|change| change.into_type()),
            Some(aloft)
        );
        assert_eq!(
            up.morphs.first().map(|change| change.into_type()),
            Some(grounded)
        );
        // Aloft it trains nothing and docks nothing: a building in transit.
        assert!(up.trainer.is_none());
        assert!(up.docks.is_empty());
        // That it is airborne is `fliers_live_on_air_alone`'s to say; here it
        // only has to be able to leave the ground at all.
        assert!(up.can_move(), "and it moves");
    }

    // The depot is the exception the name promises: it stays on the ground.
    let depot = registry
        .entity("supply_depot")
        .expect("supply_depot defined");
    assert!(depot.morphs.is_empty());
    assert!(!depot.can_move());
}

#[test]
fn planted_tank_is_priced_and_paced_like_one_that_rolls() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let planted = registry.entity("siege_tank").expect("siege_tank defined");

    assert!(
        !produced(&registry, "siege_tank"),
        "a factory rolls out tanks, not planted ones"
    );
    // 150 gold and 100 wood to train the tank, nothing more to dig in, over a
    // hundred ticks of training and sixty of planting.
    assert_eq!(
        planted.cost,
        costs::cost([("gold", 150), ("wood", 100)]),
        "an SCV bills a share of what the tank cost to have planted"
    );
    assert_eq!(planted.train_time, Some(160));
    assert!(
        planted.tags.contains("mechanical"),
        "and mends as a machine, planted or not"
    );
}

//
// ─── Undead ───────────────────────────────────────────────────────────────────
//

#[test]
fn raising_dead_names_bodies_it_raises_from() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    let raise_dead = registry
        .skill("raise_dead")
        .expect("raise_dead is registered");
    let def = registry
        .skill_def(raise_dead)
        .expect("handle came from this registry");
    let SkillCaster::Entity { target, effect, .. } = &def.caster else {
        panic!("a necromancer casts it, not the player");
    };

    // Bodies and bodies only: an unnamed filter would raise skeletons out of
    // whatever else the field came to leave lying.
    assert_eq!(
        *target,
        EntityCastTarget::Fallen {
            kinds: Kinds::only([Kind::Type("corpse".to_string())])
        }
    );
    let EntityCastEffect::Summon { entity_type, count } = effect else {
        panic!("what it does with a body is raise something from it");
    };
    assert_eq!(
        (registry.def(*entity_type).name.as_str(), *count),
        ("skeleton", 2)
    );
}

#[test]
fn hall_rising_into_next_form_still_takes_wood() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // A ghoul with a load does not wait out the change: the form a hall wears
    // while it grows accepts what the finished hall accepts.
    for rising in ["halls_of_the_dead_rising", "black_citadel_rising"] {
        let def = registry.entity(rising).expect("rising hall is registered");
        assert!(
            def.resource_storage
                .as_ref()
                .expect("rising hall stores")
                .accepts("wood")
        );
    }
}

#[test]
fn rising_hall_carries_tier_it_has_reached() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let tagged = |name: &str| {
        registry
            .entity(name)
            .expect("hall is registered")
            .tags
            .contains("grown_hall")
    };

    // A tier is reached when its growth finishes: the necropolis rising into
    // the halls of the dead has not reached it yet, while the halls rising
    // into the citadel keep the tier they already stand at — so a temple
    // ordered during that second growth is not refused.
    assert!(!tagged("necropolis"));
    assert!(!tagged("halls_of_the_dead_rising"));
    assert!(tagged("halls_of_the_dead"));
    assert!(tagged("black_citadel_rising"));
    assert!(tagged("black_citadel"));
}

#[test]
fn last_hall_is_only_one_that_fights() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let air = registry.layer(map::AIR).expect("air layer is registered");

    for quiet in ["necropolis", "halls_of_the_dead"] {
        let def = registry.entity(quiet).expect("hall is registered");
        assert!(def.attack.is_none(), "'{quiet}' answers for itself");
    }
    let citadel = registry
        .entity("black_citadel")
        .expect("black citadel is registered");
    assert!(citadel.attack.is_some());
    assert!(
        registry.targets_of(citadel) & air != 0,
        "the citadel's bolts cannot answer what flies over it"
    );
    assert!(
        citadel.trainer.is_some(),
        "the citadel trains acolytes like the halls before it"
    );
}

#[test]
fn undead_price_headroom_and_ghoul_like_other_races() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // Ten supply for eighty gold and thirty wood — eight gold a point, where
    // a farm's six for forty and twenty is under seven.
    let ziggurat = registry.entity("ziggurat").expect("ziggurat is registered");
    assert_eq!(
        ziggurat.base_stat_as_u32(EntityStatId::SUPPLY_PROVIDED),
        Some(10)
    );
    assert_eq!(ziggurat.cost, costs::cost([("gold", 80), ("wood", 30)]));

    // And the ghoul stands in the line for one supply, like every other
    // race's, swinging eight damage every eight ticks.
    let ghoul = registry.entity("ghoul").expect("ghoul is registered");
    assert_eq!(ghoul.base_stat_as_u32(EntityStatId::SUPPLY_COST), Some(1));
    assert_eq!(ghoul.base_stat_as_u32(EntityStatId::DAMAGE), Some(8));
    assert_eq!(ghoul.base_stat_as_u32(EntityStatId::ATTACK_PERIOD), Some(8));
}

#[test]
fn hardened_towers_and_grown_halls_carry_what_they_cost() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    // A ziggurat is 80 gold, 30 wood and a hundred ticks; hardening adds a
    // hundred gold for the spirit tower and 120 gold with 40 wood for the
    // nerubian one, over seventy ticks either way.
    for (tower, gold, wood) in [("spirit_tower", 180, 30), ("nerubian_tower", 200, 70)] {
        let def = registry.entity(tower).expect("tower is registered");
        assert!(
            !produced(&registry, tower),
            "'{tower}' is only hardened into"
        );
        assert_eq!(def.cost, costs::cost([("gold", gold), ("wood", wood)]));
        assert_eq!(def.build_time, Some(170));
    }

    // A necropolis is 350 gold over 180 ticks, and each growth adds 150 gold
    // over 120 more.
    for (hall, gold, time) in [("halls_of_the_dead", 500, 300), ("black_citadel", 650, 420)] {
        let def = registry.entity(hall).expect("hall is registered");
        assert!(!produced(&registry, hall), "'{hall}' is only grown into");
        assert_eq!(def.cost, costs::cost([("gold", gold)]));
        assert_eq!(def.build_time, Some(time));
    }
}

#[test]
fn haunted_mine_seats_five_acolytes_in_star() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let mine = registry
        .entity("haunted_mine")
        .expect("haunted mine is registered");

    let group = mine
        .berths
        .as_ref()
        .expect("the mine seats its acolytes")
        .group("crypt")
        .expect("the mine's berths are the crypt group");
    // Five points of a star about the mine's middle, two standing on its own
    // ground and three a half-cell outside it — one spot each, so five at
    // work stand the points rather than crowding a rim.
    assert_eq!(
        group.points(),
        [
            berth("1.0", "2.2"),
            berth("-0.1", "1.4"),
            berth("0.3", "0.0"),
            berth("1.7", "0.0"),
            berth("2.1", "1.4"),
        ]
    );
    assert_eq!(group.slots(), 5);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A berth point from decimal strings, in cells from the footprint's anchor.
fn berth(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(FixedI64::lit(x), FixedI64::lit(y))
}

/// Whether `def` refuses to stand on the named field.
fn forbids(registry: &ContentRegistry, def: &EntityTypeDef, field: &str) -> bool {
    let refused = registry.field(field).expect("field is registered");
    def.field_placement.iter().any(
        |placement| matches!(placement, FieldPlacement::Forbids { field } if *field == refused),
    )
}

/// Whether any registered type raises or trains the named one.
fn produced(registry: &ContentRegistry, name: &str) -> bool {
    registry.entities().any(|def| {
        def.builder
            .as_ref()
            .is_some_and(|builder| builder.can_build(name))
            || def
                .trainer
                .as_ref()
                .is_some_and(|trainer| trainer.can_train(name))
    })
}
