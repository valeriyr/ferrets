//! Content validation at registration: [`ContentRegistry::register`] validates
//! each definition against the content already registered and panics on any
//! inconsistency, so a referenced type must be registered before the type that
//! references it.

use ferrets_content::{
    costs::{self, Cost},
    entity_buffs::EntityBuffDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    player_stats::PlayerStatId,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    research::{ResearchDef, ResearcherDef},
    resource::{DepletionPolicy, HarvestData},
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillDef,
    },
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
    tags,
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    work::WorkPresence,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::LayerId};

//
// ─── Identity ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' is already registered")]
fn register_rejects_duplicate_type() {
    let mut registry = ground_registry();
    registry.register(worker());
    registry.register(worker());
}

//
// ─── Location ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' has no location")]
fn register_rejects_missing_location() {
    ContentRegistry::default().register(EntityTypeDef::new("worker"));
}

#[test]
fn register_accepts_square_multi_cell_mover() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("gryphon")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "entity type 'wagon' moves but has a non-square footprint")]
fn register_rejects_oblong_mover() {
    // Clearance is one number per mover and its body is a circle inscribed in
    // the footprint, so an oblong would need per-axis clearance and a rule for
    // whether the footprint turns with the mover.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::new(2, 3), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE),
    );
}

#[test]
fn register_accepts_oblong_footprint_on_something_that_cannot_move() {
    // Only movers are constrained: a 3x2 wall is a perfectly good building.
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("wall").with_location(
        GROUND,
        CellSize::new(3, 2),
        Solidity::Solid,
    ));
}

//
// ─── Resource kinds ───────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_definitions_without_resources() {
    let mut registry = ground_registry();
    registry.register(worker());
}

#[test]
fn register_accepts_registered_kinds() {
    gold_registry_with(
        worker()
            .with_cost([("gold", 10)])
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Hidden))])
            .with_resource_storage(["gold"]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its cost")]
fn register_rejects_unknown_cost_kind() {
    gold_registry_with(worker().with_cost([("wood", 10)]));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource source")]
fn register_rejects_unknown_source_kind() {
    gold_registry_with(worker().with_resource_source("wood", DepletionPolicy::Destroy));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource carrier")]
fn register_rejects_unknown_carrier_kind() {
    gold_registry_with(
        worker().with_resource_carrier([("wood", HarvestData::new(5, 2, WorkPresence::Present))]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource storage")]
fn register_rejects_unknown_storage_kind() {
    gold_registry_with(worker().with_resource_storage(["gold", "wood"]));
}

#[test]
#[should_panic(expected = "kind must not be empty")]
fn empty_resource_kind_panics() {
    ContentRegistry::default().register_resource("");
}

//
// ─── Production catalogues ────────────────────────────────────────────────────
//

// Production catalogues (trained/built types) are checked by `validate()`, not at
// registration, so they may reference each other in any order — including cycles.

#[test]
fn validate_accepts_registered_production_catalogues() {
    let mut registry = ground_registry();

    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_train_time(4),
    );
    registry.register(
        EntityTypeDef::new("depot")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_build_time(6),
    );
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_trainer(["soldier"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["depot"], WorkPresence::Hidden),
    );

    registry.validate();
}

#[test]
fn validate_accepts_production_cycle() {
    // The town hall trains the worker and the worker builds the town hall — a
    // legitimate cycle that no registration order can express, but `validate`
    // accepts because it checks against the complete registry.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("town_hall")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_build_time(6)
            .with_trainer(["worker"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_train_time(4)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["town_hall"], WorkPresence::Hidden),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'barracks' trains 'ghost', which is not a registered trainable type"
)]
fn validate_rejects_unknown_trained_type() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_trainer(["ghost"]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "trains 'statue', which is not a registered trainable type")]
fn validate_rejects_untrainable_trained_type() {
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_trainer(["statue"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'worker' builds 'nexus', which is not a registered constructible type"
)]
fn validate_rejects_unknown_built_type() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["nexus"], WorkPresence::Hidden),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "builds 'statue', which is not a registered constructible type")]
fn validate_rejects_unconstructible_built_type() {
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["statue"], WorkPresence::Hidden),
    );
    registry.validate();
}

//
// ─── Corpse chains ────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_terminating_corpse_chains() {
    let mut registry = ground_registry();

    registry.register(
        EntityTypeDef::new("bones")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(2, Some("bones")),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(3, Some("corpse")),
    );
}

#[test]
#[should_panic(expected = "entity type 'soldier' leaves an unregistered corpse type 'ghost'")]
fn register_rejects_unknown_corpse_type() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(3, Some("ghost")),
    );
}

#[test]
#[should_panic(expected = "leaves a corpse type 'statue' that has no dying phase")]
fn register_rejects_corpse_without_dying_phase() {
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(3, Some("statue")),
    );
}

#[test]
#[should_panic(
    expected = "uses 'bones' as a corpse type, but 'bones' defines live-gameplay data that remains never use"
)]
fn register_rejects_corpse_with_live_gameplay_data() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("bones")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_attack(1, 1, 1, 2, 1)
            .with_targets(GROUND)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(3, Some("bones")),
    );
}

#[test]
#[should_panic(expected = "leaves an unregistered corpse type 'bones'")]
fn register_cannot_form_corpse_cycle() {
    let mut registry = ground_registry();

    // A corpse cycle is unconstructible: a corpse type must be registered before
    // the type that leaves it, so the first member of any cycle fails because
    // its own corpse is not registered yet.
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_dying(2, Some("bones")),
    );
}

//
// ─── Race ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_registered_race() {
    let mut registry = ground_registry();
    registry.register_race("human");
    registry.register(worker().with_race("human"));
}

#[test]
#[should_panic(expected = "belongs to unregistered race 'orc'")]
fn register_rejects_unregistered_race() {
    let mut registry = ground_registry();
    registry.register(worker().with_race("orc"));
}

//
// ─── Tags ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_registered_tag() {
    let mut registry = ground_registry();
    registry.register_tag("flying");
    registry.register(worker().with_tags(["flying"]));
}

#[test]
#[should_panic(expected = "references unregistered tag 'flying'")]
fn register_rejects_unregistered_tag() {
    let mut registry = ground_registry();
    registry.register(worker().with_tags(["flying"]));
}

#[test]
#[should_panic(expected = "tag must not be empty")]
fn empty_tag_panics() {
    ContentRegistry::default().register_tag("");
}

#[test]
fn reserved_building_tag_is_registered_by_default() {
    let mut registry = ground_registry();
    assert!(registry.has_tag(tags::BUILDING));
    // Undeclared by content, yet an entity may carry it.
    registry.register(worker().with_tags([tags::BUILDING]));
}

//
// ─── Layers ───────────────────────────────────────────────────────────────────
//

#[test]
fn register_layer_assigns_ids_in_registration_order() {
    let mut registry = ContentRegistry::default();

    assert_eq!(registry.register_layer("ground"), LayerId::new(1));
    assert_eq!(registry.register_layer("air"), LayerId::new(2));
    assert_eq!(registry.register_layer("water"), LayerId::new(4));
}

#[test]
fn registered_layer_round_trips_and_keeps_its_id_on_re_registration() {
    let mut registry = ContentRegistry::default();

    let ground = registry.register_layer("ground");
    registry.register_layer("air");

    assert_eq!(registry.layer("ground"), Some(ground));
    assert!(registry.has_layer("ground"));
    assert_eq!(registry.register_layer("ground"), ground);

    assert_eq!(registry.layer("water"), None);
    assert!(!registry.has_layer("water"));
}

#[test]
#[should_panic(expected = "layer name must not be empty")]
fn empty_layer_name_panics() {
    ContentRegistry::default().register_layer("");
}

#[test]
#[should_panic(expected = "all 32 layer ids are already assigned")]
fn register_layer_rejects_exhausted_ids() {
    let mut registry = ContentRegistry::default();
    for n in 0..=32 {
        registry.register_layer(format!("layer_{n}"));
    }
}

#[test]
#[should_panic(expected = "entity type 'worker' occupies unregistered layers")]
fn register_rejects_unregistered_occupation_layer() {
    ContentRegistry::default().register(worker());
}

#[test]
fn register_accepts_occupation_of_several_registered_layers() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let air = registry.register_layer("air");
    registry.register(EntityTypeDef::new("griffon_rider").with_location(
        ground | air,
        CellSize::ONE,
        Solidity::Solid,
    ));

    let location = registry.entity("griffon_rider").unwrap().location.unwrap();
    assert_eq!(location.occupation(), ground | air);
}

#[test]
#[should_panic(expected = "entity type 'griffon_rider' occupies unregistered layers")]
fn register_rejects_occupation_mixing_in_unregistered_layer() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    registry.register(EntityTypeDef::new("griffon_rider").with_location(
        ground | LayerId::new(2),
        CellSize::ONE,
        Solidity::Solid,
    ));
}

//
// ─── Terrains ─────────────────────────────────────────────────────────────────
//

#[test]
fn register_terrain_accepts_registered_layer_masks() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let water = registry.register_layer("water");
    registry.register_terrain("grass", ground);
    registry.register_terrain("shore", ground | water);

    assert_eq!(registry.terrain("grass"), Some(ground.into()));
    assert_eq!(registry.terrain("shore"), Some(ground | water));
    assert!(registry.has_terrain("grass"));
    assert!(!registry.has_terrain("water"));
}

#[test]
fn register_terrain_accepts_impassable_terrain() {
    let mut registry = ContentRegistry::default();
    registry.register_layer("ground");
    registry.register_terrain("void", LayerMask::EMPTY);

    assert_eq!(registry.terrain("void"), Some(LayerMask::EMPTY));
}

#[test]
#[should_panic(expected = "terrain 'water' passes unregistered layers")]
fn register_terrain_rejects_unregistered_layer() {
    let mut registry = ContentRegistry::default();
    registry.register_layer("ground");
    registry.register_terrain("water", LayerId::new(2));
}

#[test]
#[should_panic(expected = "terrain 'grass' is already registered")]
fn register_terrain_rejects_duplicate_name() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    registry.register_terrain("grass", ground);
    registry.register_terrain("grass", ground);
}

#[test]
#[should_panic(expected = "terrain name must not be empty")]
fn empty_terrain_name_panics() {
    ContentRegistry::default().register_terrain("", LayerMask::EMPTY);
}

#[test]
fn validate_accepts_mover_whose_layers_one_terrain_passes_together() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let water = registry.register_layer("water");
    registry.register_terrain("grass", ground);
    registry.register_terrain("water", water);
    // A shore terrain passes both, so a shore mover has somewhere to stand.
    registry.register_terrain("shallows", ground | water);
    registry.register(
        EntityTypeDef::new("barge")
            .with_location(ground | water, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'barge' moves on layers")]
fn validate_rejects_mover_no_terrain_passes_together() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let water = registry.register_layer("water");
    registry.register_terrain("grass", ground);
    registry.register_terrain("water", water);
    // Occupation is conjunctive, so this asks for terrain passing ground *and*
    // water — which is a shore, and no terrain here is one.
    registry.register(
        EntityTypeDef::new("barge")
            .with_location(ground | water, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
fn validate_ignores_layers_of_things_that_cannot_move() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let air = registry.register_layer("air");
    registry.register_terrain("grass", ground);
    // A tall building occupies ground and air at once and never moves, so no
    // terrain has to pass the pair for it to stand where it was placed.
    registry.register(EntityTypeDef::new("tower").with_location(
        ground | air,
        CellSize::new(2, 2),
        Solidity::Solid,
    ));

    registry.validate();
}

//
// ─── Stats ──────────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "has a non-positive max_health stat")]
fn register_rejects_non_positive_max_health() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(0),
    );
}

#[test]
#[should_panic(expected = "has a non-positive speed stat")]
fn register_rejects_non_positive_speed() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ZERO, FixedU64::from_num(0.5), FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "has a non-positive supply_provided stat")]
fn register_rejects_non_positive_supply_provided() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "has a non-positive supply_cost stat")]
fn register_rejects_non_positive_supply_cost() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "has attack_range below its minimum of 1")]
fn register_rejects_zero_attack_range() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_attack(10, 0, 1, 2, 1),
    );
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_zero_attack_period() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_attack(10, 1, 1, 0, 0),
    );
}

#[test]
#[should_panic(expected = "has damage_point below its minimum of 1")]
fn register_rejects_zero_damage_point() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_attack(10, 1, 1, 2, 0),
    );
}

#[test]
#[should_panic(expected = "with an energy cost but no max_energy stat")]
fn register_rejects_costed_skill_without_energy_pool() {
    let mut registry = ground_registry();
    let jolt = registry.register_skill(
        "jolt",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: vec![EntityCastCost::Energy(FixedU64::from_num(25))],
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_skills([jolt]),
    );
}

#[test]
fn register_accepts_free_skill_without_energy_pool() {
    let mut registry = ground_registry();
    let shout = registry.register_skill(
        "shout",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_skills([shout]),
    );
    assert!(registry.entity("caster").is_some());
}

#[test]
#[should_panic(expected = "skill 'jolt' costs unregistered resource kind 'wood'")]
fn register_rejects_skill_costing_unregistered_resource() {
    let mut registry = ground_registry();
    registry.register_skill(
        "jolt",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: vec![EntityCastCost::Resources(costs::cost([("wood", 5)]))],
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_resource_costed_skill_without_pools() {
    // The stockpile is the owner's, not the type's, so a resource cost asks
    // nothing of the carrying type.
    let mut registry = ground_registry();
    registry.register_resource("gold");
    let rally = registry.register_skill(
        "rally",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: vec![EntityCastCost::Resources(costs::cost([("gold", 25)]))],
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_skills([rally]),
    );
    assert!(registry.entity("caster").is_some());
}

#[test]
#[should_panic(expected = "with a health cost but no health pool")]
fn register_rejects_health_costed_skill_without_health_pool() {
    let mut registry = ground_registry();
    let rite = registry.register_skill(
        "rite",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: vec![EntityCastCost::Health(FixedU64::from_num(5))],
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::Damage(FixedU64::from_num(5)),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_skills([rite]),
    );
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_fractional_attack_period() {
    // Positive but below one whole tick: the engine reads the cycle as an integer,
    // so this would truncate to a phase the counter never reaches.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "has a damage_point beyond its attack_period")]
fn register_rejects_damage_point_beyond_attack_period() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_attack(10, 1, 1, 2, 5)
            .with_targets(GROUND),
    );
}

#[test]
#[should_panic(expected = "entity type 'archer' has a weapon but does not declare targets")]
fn register_rejects_weapon_without_targets() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("archer")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_attack(5, 4, 4, 7, 3),
    );
}

#[test]
#[should_panic(expected = "entity type 'scarecrow' declares targets but has no weapon to aim")]
fn register_rejects_targets_without_weapon() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("scarecrow")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_targets(GROUND),
    );
}

#[test]
#[should_panic(expected = "declares health_regen without max_health")]
fn register_rejects_health_regen_without_pool() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("wall")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::HEALTH_REGEN, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "declares energy_regen without max_energy")]
fn register_rejects_energy_regen_without_pool() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("wall")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::ENERGY_REGEN, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "declares repair_speed but cannot repair")]
fn register_rejects_repair_speed_without_capability() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can build but is missing build_range")]
fn register_rejects_builder_without_reach() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_builder(["depot"], WorkPresence::Hidden),
    );
}

#[test]
#[should_panic(expected = "declares build_range but cannot build")]
fn register_rejects_build_range_without_capability() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can carry resources but is missing harvest_range")]
fn register_rejects_carrier_without_reach() {
    gold_registry_with(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Present))]),
    );
}

#[test]
#[should_panic(expected = "declares harvest_range but cannot carry resources")]
fn register_rejects_harvest_range_without_capability() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "carries the damage stat but is missing attack_range")]
fn register_rejects_attacker_without_weapon_stats() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5)),
    );
}

//
// ─── Transport ────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "can transport but is missing load_range")]
fn register_rejects_transporter_without_reach() {
    let mut registry = ground_registry();
    registry.register_tag("infantry");
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_transporter(
                ["infantry"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
}

#[test]
#[should_panic(expected = "has a non-positive cargo_capacity stat")]
fn register_rejects_zero_cargo_capacity() {
    let mut registry = ground_registry();
    registry.register_tag("infantry");
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::ZERO)
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                ["infantry"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
}

#[test]
#[should_panic(expected = "can transport but is missing cargo_capacity")]
fn register_rejects_transporter_without_capacity() {
    let mut registry = ground_registry();
    registry.register_tag("infantry");
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                ["infantry"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
}

#[test]
#[should_panic(expected = "declares unload_period but cannot transport")]
fn register_rejects_transport_stat_without_capability() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2)),
    );
}

#[test]
#[should_panic(expected = "can transport and so cannot declare cargo_size")]
fn register_rejects_transportable_transporter() {
    let mut registry = ground_registry();
    registry.register_tag("infantry");
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::from_num(2))
            .with_transporter(
                ["infantry"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
}

#[test]
#[should_panic(expected = "carries 'critters', which is not a registered entity type or tag")]
fn validate_rejects_unresolved_carries_entry() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                ["critters"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
    registry.validate();
}

#[test]
fn validate_accepts_carries_entry_registered_later() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                ["footman"],
                BoardingPolicy::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
    registry.register(
        EntityTypeDef::new("footman")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.validate();
}

//
// ─── Player stats ─────────────────────────────────────────────────────────────
//

#[test]
fn register_player_stat_resolves_built_in_name_to_its_constant() {
    let mut registry = ContentRegistry::default();

    assert_eq!(
        registry.register_player_stat("max_supply"),
        PlayerStatId::MAX_SUPPLY
    );
    assert!(registry.has_player_stat("max_supply"));
    assert_eq!(
        registry.player_stat("max_supply"),
        Some(PlayerStatId::MAX_SUPPLY)
    );
}

#[test]
fn content_declared_player_stats_get_sequential_ids_after_built_ins() {
    let mut registry = ContentRegistry::default();

    let morale = registry.register_player_stat("morale");
    let karma = registry.register_player_stat("karma");

    assert_eq!(morale.index(), PlayerStatId::MAX_SUPPLY.index() + 1);
    assert_eq!(karma.index(), PlayerStatId::MAX_SUPPLY.index() + 2);
    assert_eq!(registry.register_player_stat("morale"), morale);
    assert_eq!(registry.player_stat("karma"), Some(karma));
}

#[test]
#[should_panic(expected = "player stat name must not be empty")]
fn empty_player_stat_name_panics() {
    ContentRegistry::default().register_player_stat("");
}

#[test]
#[should_panic(expected = "'damage' is already registered as an entity stat")]
fn register_player_stat_rejects_entity_stat_name() {
    ContentRegistry::default().register_player_stat("damage");
}

#[test]
#[should_panic(expected = "'max_supply' is already registered as a player stat")]
fn register_stat_rejects_player_stat_name() {
    ContentRegistry::default().register_entity_stat("max_supply");
}

//
// ─── Skill casts ──────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_player_cast_skill() {
    let mut registry = ground_registry();
    let haste = haste_buff(&mut registry);
    let war_cry = registry.register_skill("war_cry", player_cast(haste));
    assert!(registry.has_skill("war_cry"));
    assert_eq!(registry.skill("war_cry"), Some(war_cry));
    assert_eq!(registry.skill_name(war_cry), Some("war_cry"));
}

#[test]
#[should_panic(expected = "skill 'war_cry' references an unregistered player buff")]
fn register_rejects_player_cast_skill_with_unregistered_buff() {
    // A handle from another registry names a buff this one never minted.
    let mut foreign = ContentRegistry::default();
    let buff = haste_buff(&mut foreign);
    ground_registry().register_skill("war_cry", player_cast(buff));
}

#[test]
#[should_panic(expected = "skill 'focus' references an unregistered entity buff")]
fn register_rejects_entity_cast_skill_with_unregistered_buff() {
    let mut foreign = ContentRegistry::default();
    let buff = foreign.register_entity_buff(
        "haste",
        EntityBuffDef {
            modifiers: vec![EntityModifier {
                stat: EntityStatId::SPEED,
                op: ModifierOp::PercentAdd,
                magnitude: FixedI64::ONE,
            }],
            duration: Some(10),
            stack_rule: StackRule::Refresh,
        },
    );
    ground_registry().register_skill(
        "focus",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::ApplyBuff(buff),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'war_cry' costs unregistered resource kind 'gold'")]
fn register_rejects_player_cast_skill_costing_unregistered_resource() {
    let mut registry = ground_registry();
    let haste = haste_buff(&mut registry);
    registry.register_skill(
        "war_cry",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Player {
                cost: costs::cost([("gold", 25)]),
                effect: PlayerCastEffect::ApplyBuff(haste),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "entity type 'caster' declares player-cast skill 'war_cry'")]
fn register_rejects_type_declaring_player_cast_skill() {
    let mut registry = ground_registry();
    let haste = haste_buff(&mut registry);
    let war_cry = registry.register_skill("war_cry", player_cast(haste));
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_skills([war_cry]),
    );
}

//
// ─── Repair capability ────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "can repair but is missing repair_speed")]
fn register_rejects_repairer_without_rate() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_repairer(
                ["building"],
                RepairRate::Production,
                WorkPresence::Present,
                false,
                RepairCost::Free,
                None,
            ),
    );
}

#[test]
#[should_panic(expected = "can repair but is missing repair_range")]
fn register_rejects_repairer_without_reach() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_repairer(
                ["building"],
                RepairRate::Production,
                WorkPresence::Present,
                false,
                RepairCost::Free,
                None,
            ),
    );
}

#[test]
#[should_panic(expected = "repairs unregistered tag 'mechanical'")]
fn register_rejects_repairer_mending_unknown_tag() {
    // "building" is pre-registered, so an unknown tag has to be one content would
    // have had to declare itself.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                ["mechanical"],
                RepairRate::Production,
                WorkPresence::Present,
                false,
                RepairCost::Free,
                None,
            ),
    );
}

#[test]
#[should_panic(expected = "charges pro-rata repair but is missing repair_cost_factor")]
fn register_rejects_pro_rata_repair_without_factor() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                ["building"],
                RepairRate::Production,
                WorkPresence::Present,
                false,
                RepairCost::ProRata,
                None,
            ),
    );
}

#[test]
#[should_panic(expected = "pays for repair with energy but has no max_energy stat")]
fn register_rejects_energy_paid_repair_without_pool() {
    let mut registry = ground_registry();
    registry.register_tag("biological");
    registry.register(
        EntityTypeDef::new("medic")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::from_num(2))
            .with_repairer(
                ["biological"],
                RepairRate::PerTick(FixedU64::ONE),
                WorkPresence::Present,
                false,
                RepairCost::Energy(FixedU64::from_num(0.5)),
                None,
            ),
    );
}

#[test]
#[should_panic(expected = "a flat repair rate must be positive")]
fn repairer_rejects_non_positive_flat_rate() {
    EntityTypeDef::new("medic").with_repairer(
        ["biological"],
        RepairRate::PerTick(FixedU64::ZERO),
        WorkPresence::Present,
        false,
        RepairCost::Free,
        None,
    );
}

#[test]
#[should_panic(expected = "has a repair_ratio but no build_time or train_time")]
fn register_rejects_repair_ratio_without_production_time() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("monolith")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(100)
            .with_repair_ratio(FixedU64::ONE),
    );
}

//
// ─── Research ─────────────────────────────────────────────────────────────────
//

#[test]
fn register_research_assigns_ids_and_resolves_names() {
    let mut registry = ground_registry();
    registry.register_resource("gold");
    let buff = haste_buff(&mut registry);

    let smithing = registry.register_research(
        "smithing",
        ResearchDef::new(
            costs::cost([("gold", 30)]),
            10,
            Some(buff),
            Vec::<String>::new(),
        ),
    );
    let tactics = registry.register_research(
        "tactics",
        ResearchDef::new(Cost::new(), 5, None, ["smithing"]),
    );

    assert!(registry.has_research("smithing"));
    assert_eq!(registry.research("smithing"), Some(smithing));
    assert_eq!(registry.research_name(tactics), Some("tactics"));
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
    assert_eq!(registry.research_def(tactics).unwrap().buff, None);

    // Re-registering a name keeps the first definition and returns its id.
    let again =
        registry.register_research("smithing", ResearchDef::new(Cost::new(), 99, None, ["x"]));
    assert_eq!(again, smithing);
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
}

#[test]
#[should_panic(expected = "research name must not be empty")]
fn register_research_rejects_empty_name() {
    ground_registry().register_research("", ResearchDef::new(Cost::new(), 10, None, ["worker"]));
}

#[test]
#[should_panic(expected = "research 'smithing' costs unregistered resource kind 'gold'")]
fn register_research_rejects_unknown_cost_kind() {
    ground_registry().register_research(
        "smithing",
        ResearchDef::new(costs::cost([("gold", 30)]), 10, None, Vec::<String>::new()),
    );
}

#[test]
#[should_panic(expected = "research 'smithing' references an unregistered player buff")]
fn register_research_rejects_unregistered_buff() {
    // A handle from another registry names a buff this one never minted.
    let mut foreign = ContentRegistry::default();
    let buff = haste_buff(&mut foreign);
    ground_registry().register_research(
        "smithing",
        ResearchDef::new(Cost::new(), 10, Some(buff), Vec::<String>::new()),
    );
}

#[test]
#[should_panic(expected = "research_time must be greater than 0")]
fn research_def_rejects_zero_time() {
    ResearchDef::new(Cost::new(), 0, None, Vec::<String>::new());
}

#[test]
#[should_panic(expected = "requirement names must not be empty")]
fn research_def_rejects_empty_requirement_name() {
    ResearchDef::new(Cost::new(), 10, None, [""]);
}

#[test]
#[should_panic(expected = "researches must not be empty")]
fn researcher_def_rejects_empty_catalogue() {
    ResearcherDef::new([]);
}

#[test]
#[should_panic(expected = "entity type 'lab' hosts an unregistered research")]
fn register_rejects_unregistered_hosted_research() {
    let mut foreign = ContentRegistry::default();
    let research =
        foreign.register_research("smithing", ResearchDef::new(Cost::new(), 10, None, ["x"]));
    ground_registry().register(
        EntityTypeDef::new("lab")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_researcher([research]),
    );
}

//
// ─── Requirements ─────────────────────────────────────────────────────────────
//

// Requirement lists are forward references, checked by `validate()` against the
// complete registry: each entry must name exactly one of an entity type, a tag,
// or a research.

#[test]
fn validate_accepts_type_tag_and_research_requirements() {
    let mut registry = ground_registry();
    let smithing =
        registry.register_research("smithing", ResearchDef::new(Cost::new(), 10, None, ["lab"]));
    // The knight's requirements name a type registered after it, the reserved
    // "building" tag, and a research.
    registry.register(
        EntityTypeDef::new("knight")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_requires(["lab", tags::BUILDING, "smithing"]),
    );
    registry.register(
        EntityTypeDef::new("lab")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_researcher([smithing]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires 'chapel', which is not a registered entity type, tag, or research"
)]
fn validate_rejects_unknown_requirement() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("knight")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_requires(["chapel"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires 'forge', which names both a research and an entity type or tag"
)]
fn validate_rejects_ambiguous_requirement() {
    let mut registry = ground_registry();
    registry.register_research(
        "forge",
        ResearchDef::new(Cost::new(), 10, None, Vec::<String>::new()),
    );
    registry.register(EntityTypeDef::new("forge").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("knight")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_requires(["forge"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "research 'smithing' requires 'chapel', which is not a registered entity type, tag, or research"
)]
fn validate_rejects_unknown_research_requirement() {
    let mut registry = ground_registry();
    registry.register_research(
        "smithing",
        ResearchDef::new(Cost::new(), 10, None, ["chapel"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "skill 'war_cry' requires 'chapel', which is not a registered entity type, tag, or research"
)]
fn validate_rejects_unknown_skill_requirement() {
    let mut registry = ground_registry();
    let haste = haste_buff(&mut registry);
    let mut skill = player_cast(haste);
    skill.requires = vec!["chapel".to_string()];
    registry.register_skill("war_cry", skill);
    registry.validate();
}

#[test]
fn validate_accepts_research_requirement_on_skill() {
    let mut registry = ground_registry();
    let haste = haste_buff(&mut registry);
    registry.register_research(
        "war_drums",
        ResearchDef::new(Cost::new(), 10, None, Vec::<String>::new()),
    );
    let mut skill = player_cast(haste);
    skill.requires = vec!["war_drums".to_string()];
    registry.register_skill("war_cry", skill);
    registry.validate();
}

//
// ─── Morph transitions ────────────────────────────────────────────────────────
//

#[test]
fn validate_accepts_transitions_naming_each_other() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE)
            .with_morphs([morph_into("flier")]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE)
            .with_morphs([morph_into("walker")]),
    );

    registry.validate();
}

#[test]
fn validate_accepts_one_way_transition() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([morph_into("flier")]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' names a type that is not registered"
)]
fn validate_rejects_transition_into_unregistered_type() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([morph_into("flier")]),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "odd footprint difference")]
fn validate_rejects_transition_with_odd_footprint_difference() {
    // Recentring shifts the anchor by half the size difference per axis: a
    // 1x1 -> 2x2 transition would land it between lattice points.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([morph_into("giant")]),
    );
    registry.register(
        EntityTypeDef::new("giant")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE),
    );

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_even_footprint_difference() {
    // 1x1 -> 3x3 recentres by a whole cell per axis, which stays on lattice.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([morph_into("giant")]),
    );
    registry.register(
        EntityTypeDef::new("giant")
            .with_location(GROUND, CellSize::new(3, 3), Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::ONE, FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' requires 'jet_pack', which is \
                not a registered entity type, tag, or research"
)]
fn validate_rejects_transition_with_unresolved_requirement() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([MorphTransition::new(
                "flier",
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                Vec::new(),
                ["jet_pack"],
            )]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' reads its time from a stat the \
                type does not carry"
)]
fn validate_rejects_transition_timed_by_undeclared_stat() {
    let mut registry = ground_registry();
    let stat = registry.register_entity_stat("change_time");
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([MorphTransition::new(
                "flier",
                MorphTime::Stat(stat),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                Vec::new(),
                Vec::<String>::new(),
            )]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' has an energy cost but no \
                max_energy stat"
)]
fn validate_rejects_transition_with_energy_cost_but_no_energy_pool() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([MorphTransition::new(
                "flier",
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                vec![EntityCastCost::Energy(FixedU64::from_num(20))],
                Vec::<String>::new(),
            )]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' costs unregistered resource \
                kind 'gold'"
)]
fn validate_rejects_transition_with_unregistered_resource_cost() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_morphs([MorphTransition::new(
                "flier",
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                vec![EntityCastCost::Resources(costs::cost([("gold", 50)]))],
                Vec::<String>::new(),
            )]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_payable_costs() {
    let mut registry = ground_registry();
    registry.register_resource("gold");
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE)
            .with_energy(100, FixedU64::from_num(0.1))
            .with_morphs([MorphTransition::new(
                "flier",
                MorphTime::Constant(20),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                vec![
                    EntityCastCost::Resources(costs::cost([("gold", 50)])),
                    EntityCastCost::Energy(FixedU64::from_num(20)),
                ],
                Vec::<String>::new(),
            )]),
    );
    registry.register(
        EntityTypeDef::new("flier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ONE, FixedU64::from_num(0.5), FixedU64::ONE),
    );

    registry.validate();
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

const GROUND: LayerId = LayerId::new(1);

/// A fresh registry that already knows the "ground" navigation layer.
fn ground_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::default();
    registry.register_layer("ground");
    registry
}

/// Registers `def` into a registry that already knows the "gold" resource kind.
fn gold_registry_with(def: EntityTypeDef) {
    let mut registry = ground_registry();
    registry.register_resource("gold");
    registry.register(def);
}

fn worker() -> EntityTypeDef {
    EntityTypeDef::new("worker").with_location(GROUND, CellSize::ONE, Solidity::Solid)
}

/// A player-cast buff skill.
fn player_cast(buff: PlayerBuffId) -> SkillDef {
    SkillDef {
        cooldown: 10,
        caster: SkillCaster::Player {
            cost: Cost::new(),
            effect: PlayerCastEffect::ApplyBuff(buff),
        },
        requires: Vec::new(),
    }
}

/// An army-wide speed buff registered into `registry`.
fn haste_buff(registry: &mut ContentRegistry) -> PlayerBuffId {
    registry.register_player_buff(
        "haste",
        PlayerBuffDef {
            player_modifiers: Vec::new(),
            entity_modifiers: vec![EntityModifier {
                stat: EntityStatId::SPEED,
                op: ModifierOp::PercentAdd,
                magnitude: FixedI64::ONE,
            }],
            duration: Some(10),
            stack_rule: StackRule::Refresh,
        },
    )
}

/// A free, timed, committed transition into the named type.
fn morph_into(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphTime::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        Vec::new(),
        Vec::<String>::new(),
    )
}
