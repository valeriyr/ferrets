//! Content validation at registration: [`ContentRegistry::register`] validates
//! each definition against the content already registered and panics on any
//! inconsistency, so a referenced type must be registered before the type that
//! references it.

mod utils;

use ferrets_content::{
    attack::{Delivery, Weapon},
    build::BuilderAttendance,
    costs::{self, Cost},
    entity_buffs::EntityBuffDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        FieldAction, FieldAffiliation, FieldCoverage, FieldDecay, FieldDef, FieldEffect,
        FieldEffectKind, FieldGrowth, FieldId, FieldPlacement, FieldSide, FieldSourceDef,
        FieldVision,
    },
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
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp},
    tags,
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::WorkPresence,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};
use utils::GROUND;

//
// ─── Identity ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' is already registered")]
fn register_rejects_duplicate_type() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND));
    registry.register(utils::standing("worker", GROUND));
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("gryphon", GROUND, CellSize::new(2, 2)).with_movement(
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );
}

#[test]
#[should_panic(expected = "entity type 'wagon' moves but has a non-square footprint")]
fn register_rejects_oblong_mover() {
    // Clearance is one number per mover and its body is a circle inscribed in
    // the footprint, so an oblong would need per-axis clearance and a rule for
    // whether the footprint turns with the mover.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("wagon", GROUND, CellSize::new(2, 3)).with_movement(
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );
}

#[test]
fn register_accepts_oblong_footprint_on_something_that_cannot_move() {
    // Only movers are constrained: a 3x2 wall is a perfectly good building.
    let mut registry = utils::ground_registry();
    registry.register(utils::sized("wall", GROUND, CellSize::new(3, 2)));
}

//
// ─── Resource kinds ───────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_definitions_without_resources() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND));
}

#[test]
fn register_accepts_registered_kinds() {
    gold_registry_with(
        utils::standing("worker", GROUND)
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
    gold_registry_with(utils::standing("worker", GROUND).with_cost([("wood", 10)]));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource source")]
fn register_rejects_unknown_source_kind() {
    gold_registry_with(
        utils::standing("worker", GROUND).with_resource_source("wood", DepletionPolicy::Destroy),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource carrier")]
fn register_rejects_unknown_carrier_kind() {
    gold_registry_with(
        utils::standing("worker", GROUND)
            .with_resource_carrier([("wood", HarvestData::new(5, 2, WorkPresence::Present))]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource storage")]
fn register_rejects_unknown_storage_kind() {
    gold_registry_with(utils::standing("worker", GROUND).with_resource_storage(["gold", "wood"]));
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
    let mut registry = utils::ground_registry();

    registry.register(utils::standing("soldier", GROUND).with_train_time(4));
    registry.register(utils::sized("depot", GROUND, CellSize::new(2, 2)).with_build_time(6));
    registry
        .register(utils::sized("barracks", GROUND, CellSize::new(2, 2)).with_trainer(["soldier"]));
    registry.register(
        utils::standing("worker", GROUND)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["depot"], BuilderAttendance::Crew(WorkPresence::Hidden)),
    );

    registry.validate();
}

#[test]
fn validate_accepts_production_cycle() {
    // The town hall trains the worker and the worker builds the town hall — a
    // legitimate cycle that no registration order can express, but `validate`
    // accepts because it checks against the complete registry.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("town_hall", GROUND, CellSize::new(2, 2))
            .with_build_time(6)
            .with_trainer(["worker"]),
    );
    registry.register(
        utils::standing("worker", GROUND)
            .with_train_time(4)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["town_hall"], BuilderAttendance::Crew(WorkPresence::Hidden)),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'barracks' trains 'ghost', which is not a registered trainable type"
)]
fn validate_rejects_unknown_trained_type() {
    let mut registry = utils::ground_registry();
    registry
        .register(utils::sized("barracks", GROUND, CellSize::new(2, 2)).with_trainer(["ghost"]));
    registry.validate();
}

#[test]
#[should_panic(expected = "trains 'statue', which is not a registered trainable type")]
fn validate_rejects_untrainable_trained_type() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry
        .register(utils::sized("barracks", GROUND, CellSize::new(2, 2)).with_trainer(["statue"]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'worker' builds 'nexus', which is not a registered constructible type"
)]
fn validate_rejects_unknown_built_type() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["nexus"], BuilderAttendance::Crew(WorkPresence::Hidden)),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "builds 'statue', which is not a registered constructible type")]
fn validate_rejects_unconstructible_built_type() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        utils::standing("worker", GROUND)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(["statue"], BuilderAttendance::Crew(WorkPresence::Hidden)),
    );
    registry.validate();
}

//
// ─── Corpse chains ────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_terminating_corpse_chains() {
    let mut registry = utils::ground_registry();

    registry.register(utils::standing("bones", GROUND).with_dying(2, None));
    registry.register(utils::standing("corpse", GROUND).with_dying(2, Some("bones")));
    registry.register(utils::standing("soldier", GROUND).with_dying(3, Some("corpse")));
}

#[test]
#[should_panic(expected = "entity type 'soldier' leaves an unregistered corpse type 'ghost'")]
fn register_rejects_unknown_corpse_type() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("soldier", GROUND).with_dying(3, Some("ghost")));
}

#[test]
#[should_panic(expected = "leaves a corpse type 'statue' that has no dying phase")]
fn register_rejects_corpse_without_dying_phase() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(utils::standing("soldier", GROUND).with_dying(3, Some("statue")));
}

#[test]
#[should_panic(
    expected = "uses 'bones' as a corpse type, but 'bones' defines live-gameplay data that remains never use"
)]
fn register_rejects_corpse_with_live_gameplay_data() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("bones", GROUND)
            .with_health(10)
            .with_attack(utils::weapon(GROUND), 1, 1, 1, 2, 1)
            .with_dying(2, None),
    );
    registry.register(utils::standing("soldier", GROUND).with_dying(3, Some("bones")));
}

#[test]
#[should_panic(expected = "leaves an unregistered corpse type 'bones'")]
fn register_cannot_form_corpse_cycle() {
    let mut registry = utils::ground_registry();

    // A corpse cycle is unconstructible: a corpse type must be registered before
    // the type that leaves it, so the first member of any cycle fails because
    // its own corpse is not registered yet.
    registry.register(utils::standing("corpse", GROUND).with_dying(2, Some("bones")));
}

//
// ─── Race ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_registered_race() {
    let mut registry = utils::ground_registry();
    registry.register_race("human");
    registry.register(utils::standing("worker", GROUND).with_race("human"));
}

#[test]
#[should_panic(expected = "belongs to unregistered race 'orc'")]
fn register_rejects_unregistered_race() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_race("orc"));
}

//
// ─── Tags ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_registered_tag() {
    let mut registry = utils::ground_registry();
    registry.register_tag("flying");
    registry.register(utils::standing("worker", GROUND).with_tags(["flying"]));
}

#[test]
#[should_panic(expected = "references unregistered tag 'flying'")]
fn register_rejects_unregistered_tag() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_tags(["flying"]));
}

#[test]
#[should_panic(expected = "tag must not be empty")]
fn empty_tag_panics() {
    ContentRegistry::default().register_tag("");
}

#[test]
fn reserved_building_tag_is_registered_by_default() {
    let mut registry = utils::ground_registry();
    assert!(registry.has_tag(tags::BUILDING));
    // Undeclared by content, yet an entity may carry it.
    registry.register(utils::standing("worker", GROUND).with_tags([tags::BUILDING]));
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
    ContentRegistry::default().register(utils::standing("worker", GROUND));
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
    registry.register(utils::standing("barge", ground | water).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

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
    registry.register(utils::standing("barge", ground | water).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

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
    registry.register(utils::sized("tower", ground | air, CellSize::new(2, 2)));

    registry.validate();
}

//
// ─── Stats ──────────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "has a non-positive max_health stat")]
fn register_rejects_non_positive_max_health() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_health(0));
}

#[test]
#[should_panic(expected = "has a non-positive speed stat")]
fn register_rejects_non_positive_speed() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_movement(
        FixedU64::ZERO,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));
}

#[test]
#[should_panic(expected = "has a non-positive supply_provided stat")]
fn register_rejects_non_positive_supply_provided() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND).with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "has a non-positive supply_cost stat")]
fn register_rejects_non_positive_supply_cost() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND).with_stat(EntityStatId::SUPPLY_COST, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "declares aim_rate but carries no turret")]
fn register_rejects_aim_rate_without_turret() {
    // Only a gun with a bearing of its own reads a slew rate; on a body that turns
    // to shoot it is a rate its author believes in and nothing applies.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("gunner", GROUND)
            .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(3))
            .with_attack(utils::weapon(GROUND), 10, 1, 1, 2, 1),
    );
}

#[test]
#[should_panic(expected = "declares attack_arc but has no weapon")]
fn register_rejects_attack_arc_without_weapon() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", GROUND).with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(60)),
    );
}

#[test]
#[should_panic(expected = "declares pivot_angle but cannot move")]
fn register_rejects_pivot_angle_without_movement() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("keep", GROUND)
            .with_stat(EntityStatId::PIVOT_ANGLE, FixedU64::from_num(90)),
    );
}

#[test]
#[should_panic(expected = "has attack_range below its minimum of 1")]
fn register_rejects_zero_attack_range() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_attack(
        utils::weapon(GROUND),
        10,
        0,
        1,
        2,
        1,
    ));
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_zero_attack_period() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_attack(
        utils::weapon(GROUND),
        10,
        1,
        1,
        0,
        0,
    ));
}

#[test]
#[should_panic(expected = "has damage_point below its minimum of 1")]
fn register_rejects_zero_damage_point() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_attack(
        utils::weapon(GROUND),
        10,
        1,
        1,
        2,
        0,
    ));
}

#[test]
#[should_panic(expected = "with an energy cost but no max_energy stat")]
fn register_rejects_costed_skill_without_energy_pool() {
    let mut registry = utils::ground_registry();
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
        utils::standing("caster", GROUND)
            .with_health(20)
            .with_skills([jolt]),
    );
}

#[test]
fn register_accepts_free_skill_without_energy_pool() {
    let mut registry = utils::ground_registry();
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
        utils::standing("caster", GROUND)
            .with_health(20)
            .with_skills([shout]),
    );
    assert!(registry.entity("caster").is_some());
}

#[test]
#[should_panic(expected = "skill 'jolt' costs unregistered resource kind 'wood'")]
fn register_rejects_skill_costing_unregistered_resource() {
    let mut registry = utils::ground_registry();
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
    let mut registry = utils::ground_registry();
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
        utils::standing("caster", GROUND)
            .with_health(20)
            .with_skills([rally]),
    );
    assert!(registry.entity("caster").is_some());
}

#[test]
#[should_panic(expected = "with a health cost but no health pool")]
fn register_rejects_health_costed_skill_without_health_pool() {
    let mut registry = utils::ground_registry();
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
    registry.register(utils::standing("caster", GROUND).with_skills([rite]));
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_fractional_attack_period() {
    // Positive but below one whole tick: the engine reads the cycle as an integer,
    // so this would truncate to a phase the counter never reaches.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "has a damage_point beyond its attack_period")]
fn register_rejects_damage_point_beyond_attack_period() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", GROUND).with_attack(
        utils::weapon(GROUND),
        10,
        1,
        1,
        2,
        5,
    ));
}

#[test]
#[should_panic(expected = "entity type 'archer' declares damage but has no weapon")]
fn register_rejects_weapon_numbers_without_weapon() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("archer", GROUND)
            // Stat by stat rather than through `with_attack`, which cannot state
            // a weapon's numbers without the weapon — the very thing under test.
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(7))
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3)),
    );
}

#[test]
#[should_panic(expected = "entity type 'scarecrow' points a weapon but is missing damage")]
fn register_rejects_weapon_without_its_numbers() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("scarecrow", GROUND)
            .with_health(10)
            .with_attack_def(GROUND, Delivery::Instant, None),
    );
}

#[test]
#[should_panic(expected = "declares health_regen without max_health")]
fn register_rejects_health_regen_without_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", GROUND)
            .with_stat(EntityStatId::HEALTH_REGEN, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "declares energy_regen without max_energy")]
fn register_rejects_energy_regen_without_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", GROUND)
            .with_health(20)
            .with_stat(EntityStatId::ENERGY_REGEN, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "declares repair_speed but cannot repair")]
fn register_rejects_repair_speed_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can build but is missing build_range")]
fn register_rejects_builder_without_reach() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_builder(["depot"], BuilderAttendance::Crew(WorkPresence::Hidden)),
    );
}

#[test]
#[should_panic(expected = "declares build_range but cannot build")]
fn register_rejects_build_range_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", GROUND).with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can carry resources but is missing harvest_range")]
fn register_rejects_carrier_without_reach() {
    gold_registry_with(
        utils::standing("worker", GROUND)
            .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Present))]),
    );
}

#[test]
#[should_panic(expected = "declares harvest_range but cannot carry resources")]
fn register_rejects_harvest_range_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", GROUND).with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "points a weapon but is missing attack_range")]
fn register_rejects_attacker_without_weapon_stats() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
            .with_attack_def(GROUND, Delivery::Instant, None),
    );
}

//
// ─── Transport ────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "can transport but is missing load_range")]
fn register_rejects_transporter_without_reach() {
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", GROUND)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2)),
    );
}

#[test]
#[should_panic(expected = "can transport and so cannot declare cargo_size")]
fn register_rejects_transportable_transporter() {
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wagon", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wagon", GROUND)
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
        utils::standing("footman", GROUND).with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
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
    let mut registry = utils::ground_registry();
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
    utils::ground_registry().register_skill("war_cry", player_cast(buff));
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
    utils::ground_registry().register_skill(
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
    let mut registry = utils::ground_registry();
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
    let mut registry = utils::ground_registry();
    let haste = haste_buff(&mut registry);
    let war_cry = registry.register_skill("war_cry", player_cast(haste));
    registry.register(
        utils::standing("caster", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register_tag("biological");
    registry.register(
        utils::standing("medic", GROUND)
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
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("monolith", GROUND)
            .with_health(100)
            .with_repair_ratio(FixedU64::ONE),
    );
}

//
// ─── Research ─────────────────────────────────────────────────────────────────
//

#[test]
fn register_research_assigns_ids_and_resolves_names() {
    let mut registry = utils::ground_registry();
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
    utils::ground_registry()
        .register_research("", ResearchDef::new(Cost::new(), 10, None, ["worker"]));
}

#[test]
#[should_panic(expected = "research 'smithing' costs unregistered resource kind 'gold'")]
fn register_research_rejects_unknown_cost_kind() {
    utils::ground_registry().register_research(
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
    utils::ground_registry().register_research(
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
    utils::ground_registry().register(utils::standing("lab", GROUND).with_researcher([research]));
}

//
// ─── Requirements ─────────────────────────────────────────────────────────────
//

// Requirement lists are forward references, checked by `validate()` against the
// complete registry: each entry must name exactly one of an entity type, a tag,
// or a research.

#[test]
fn validate_accepts_type_tag_and_research_requirements() {
    let mut registry = utils::ground_registry();
    let smithing =
        registry.register_research("smithing", ResearchDef::new(Cost::new(), 10, None, ["lab"]));
    // The knight's requirements name a type registered after it, the reserved
    // "building" tag, and a research.
    registry.register(utils::standing("knight", GROUND).with_requires([
        "lab",
        tags::BUILDING,
        "smithing",
    ]));
    registry.register(utils::standing("lab", GROUND).with_researcher([smithing]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires 'chapel', which is not a registered entity type, tag, or research"
)]
fn validate_rejects_unknown_requirement() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("knight", GROUND).with_requires(["chapel"]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires 'forge', which names both a research and an entity type or tag"
)]
fn validate_rejects_ambiguous_requirement() {
    let mut registry = utils::ground_registry();
    registry.register_research(
        "forge",
        ResearchDef::new(Cost::new(), 10, None, Vec::<String>::new()),
    );
    registry.register(EntityTypeDef::new("forge").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(utils::standing("knight", GROUND).with_requires(["forge"]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "research 'smithing' requires 'chapel', which is not a registered entity type, tag, or research"
)]
fn validate_rejects_unknown_research_requirement() {
    let mut registry = utils::ground_registry();
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
    let mut registry = utils::ground_registry();
    let haste = haste_buff(&mut registry);
    let mut skill = player_cast(haste);
    skill.requires = vec!["chapel".to_string()];
    registry.register_skill("war_cry", skill);
    registry.validate();
}

#[test]
fn validate_accepts_research_requirement_on_skill() {
    let mut registry = utils::ground_registry();
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

#[test]
fn validate_accepts_bonus_against_type_or_tag() {
    // Either kind resolves, and a bonus may name a type registered after the type
    // that fears it — which is why this is judged after registration, not during.
    let mut registry = utils::ground_registry();
    registry.register_tag("armored");
    registry.register(
        utils::standing("archer", GROUND)
            .with_bonus_damage_vs([("armored", 4u32), ("keep", 6u32)])
            .with_attack(utils::weapon(GROUND), 6, 4, 4, 7, 3),
    );
    registry.register(utils::standing("keep", GROUND));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'archer' deals bonus damage to 'sieging', which is not a registered entity type or tag"
)]
fn validate_rejects_bonus_against_unknown_name() {
    // A typo here is a bonus that silently never applies, which is the quietest
    // way content can be wrong.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("archer", GROUND)
            .with_bonus_damage_vs([("sieging", 4u32)])
            .with_attack(utils::weapon(GROUND), 6, 4, 4, 7, 3),
    );

    registry.validate();
}

//
// ─── Morph transitions ────────────────────────────────────────────────────────
//

#[test]
fn validate_accepts_transitions_naming_each_other() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("walker", GROUND, CellSize::new(2, 2))
            .with_movement(
                FixedU64::ONE,
                FixedU64::ONE,
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("flier")]),
    );
    registry.register(
        utils::sized("flier", GROUND, CellSize::new(2, 2))
            .with_movement(
                FixedU64::ONE,
                FixedU64::ONE,
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("walker")]),
    );

    registry.validate();
}

#[test]
fn validate_accepts_one_way_transition() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("flier")]),
    );
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' names a type that is not registered"
)]
fn validate_rejects_transition_into_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("flier")]),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "odd footprint difference")]
fn validate_rejects_transition_with_odd_footprint_difference() {
    // Recentring shifts the anchor by half the size difference per axis: a
    // 1x1 -> 2x2 transition would land it between lattice points.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("giant")]),
    );
    registry.register(
        utils::sized("giant", GROUND, CellSize::new(2, 2)).with_movement(
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_even_footprint_difference() {
    // 1x1 -> 3x3 recentres by a whole cell per axis, which stays on lattice.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("giant")]),
    );
    registry.register(
        utils::sized("giant", GROUND, CellSize::new(3, 3)).with_movement(
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' requires 'jet_pack', which is \
                not a registered entity type, tag, or research"
)]
fn validate_rejects_transition_with_unresolved_requirement() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                None,
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                Vec::new(),
                ["jet_pack"],
            )]),
    );
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' reads its time from a stat the \
                type does not carry"
)]
fn validate_rejects_transition_timed_by_undeclared_stat() {
    let mut registry = utils::ground_registry();
    let stat = registry.register_entity_stat("change_time");
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                None,
                MorphTime::Stat(stat),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                Vec::new(),
                Vec::<String>::new(),
            )]),
    );
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' has an energy cost but no \
                max_energy stat"
)]
fn validate_rejects_transition_with_energy_cost_but_no_energy_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                None,
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                vec![EntityCastCost::Energy(FixedU64::from_num(20))],
                Vec::<String>::new(),
            )]),
    );
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' costs unregistered resource \
                kind 'gold'"
)]
fn validate_rejects_transition_with_unregistered_resource_cost() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                None,
                MorphTime::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                vec![EntityCastCost::Resources(costs::cost([("gold", 50)]))],
                Vec::<String>::new(),
            )]),
    );
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

#[test]
#[should_panic(expected = "wears a form that is not registered")]
fn validate_rejects_transition_through_unregistered_form() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", GROUND)
            .with_health(20)
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(utils::standing("hatchling", GROUND).with_health(30));

    registry.validate();
}

#[test]
#[should_panic(expected = "wears a form whose footprint differs from its own")]
fn validate_rejects_transition_through_form_of_other_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", GROUND)
            .with_health(20)
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(utils::sized("egg", GROUND, CellSize::new(2, 2)).with_health(60));
    registry.register(utils::standing("hatchling", GROUND).with_health(30));

    registry.validate();
}

#[test]
fn validate_accepts_transition_through_form_of_same_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", GROUND)
            .with_health(20)
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(utils::standing("egg", GROUND).with_health(60));
    registry.register(utils::standing("hatchling", GROUND).with_health(30));

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_payable_costs() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("walker", GROUND)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_energy(100, FixedU64::from_num(0.1))
            .with_morphs([MorphTransition::new(
                "flier",
                None,
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
    registry.register(utils::standing("flier", GROUND).with_movement(
        FixedU64::ONE,
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));

    registry.validate();
}

//
// ─── Turrets ──────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(
    expected = "entity type 'bunker' carries a turret that fires on the move but cannot move"
)]
fn validate_rejects_turret_firing_while_moving_without_movement() {
    let mut registry = utils::ground_registry();
    let rolling = registry.register_turret(
        "rolling",
        TurretDef::new(
            Weapon::new(GROUND, Delivery::Instant, None),
            TurretStats::default(),
            WeaponConduct::OnTheMove,
        ),
    );
    registry.register(
        utils::standing("bunker", GROUND)
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
            .with_turrets([TurretMount::new(rolling, CellPos::new(0, 0), CellSize::ONE)]),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' mounts a turret outside its own footprint")]
fn validate_rejects_turret_mounted_off_its_footprint() {
    let mut registry = utils::ground_registry();
    let gun = registry.register_turret(
        "gun",
        TurretDef::new(
            Weapon::new(GROUND, Delivery::Instant, None),
            TurretStats::default(),
            WeaponConduct::Halts,
        ),
    );
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
            .with_turrets([TurretMount::new(
                gun,
                CellPos::new(1, 1),
                CellSize::new(2, 2),
            )]),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'keep' carries a turret whose damage reads damage, which it does not \
                declare"
)]
fn validate_rejects_turret_reading_stat_its_body_lacks() {
    let mut registry = utils::ground_registry();
    let gun = registry.register_turret(
        "gun",
        TurretDef::new(
            Weapon::new(GROUND, Delivery::Instant, None),
            TurretStats::default(),
            WeaponConduct::Halts,
        ),
    );
    registry.register(
        utils::standing("keep", GROUND).with_turrets([TurretMount::new(
            gun,
            CellPos::new(0, 0),
            CellSize::ONE,
        )]),
    );

    registry.validate();
}

//
// ─── Fields ───────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_field_sources_placement_and_effects() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", GROUND)
            .with_field_sources([emitter(creep)])
            .with_field_placement([FieldPlacement::Requires {
                field: creep,
                of: FieldAffiliation::Anyone,
                coverage: FieldCoverage::Footprint,
            }])
            .with_field_effects([FieldEffect::new(
                creep,
                FieldAffiliation::Anyone,
                FieldSide::Inside,
                FieldEffectKind::Modifiers(vec![EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                }]),
            )]),
    );
    registry.validate();

    assert_eq!(registry.field("creep"), Some(creep));
    assert_eq!(registry.field_name(creep), Some("creep"));
    assert_eq!(
        registry.field_def(creep).decay(),
        FieldDecay::Gradual { cycle: 4 }
    );
    assert_eq!(registry.field_def(creep).vision(), FieldVision::Dark);
    assert_eq!(registry.field_ids().collect::<Vec<_>>(), vec![creep]);
}

#[test]
#[should_panic(expected = "acts on an unregistered field when it stands")]
fn validate_rejects_standing_act_on_unregistered_field() {
    let mut registry = utils::ground_registry();
    // A handle minted by another registry, which this one never registered.
    let foreign = utils::ground_registry().register_field(
        "blight",
        FieldDef::new(GROUND, FieldDecay::Never, FieldVision::Dark),
    );
    registry.register(
        utils::standing("pylon", GROUND)
            .with_health(100)
            .with_standing_acts([StandingAct::Field {
                field: foreign,
                radius: 3,
                action: FieldAction::Clear,
            }]),
    );
    registry.validate();
}

#[test]
fn register_accepts_standing_act_on_registered_field() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("pylon", GROUND)
            .with_health(100)
            .with_standing_acts([StandingAct::Field {
                field: creep,
                radius: 3,
                action: FieldAction::Clear,
            }]),
    );
    registry.validate();

    let pylon = registry.entity("pylon").expect("pylon is registered");
    assert_eq!(
        pylon.on_stand,
        vec![StandingAct::Field {
            field: creep,
            radius: 3,
            action: FieldAction::Clear,
        }]
    );
}

#[test]
fn register_keeps_field_vision() {
    let mut registry = utils::ground_registry();
    let watched = registry.register_field(
        "creep",
        FieldDef::new(GROUND, FieldDecay::Never, FieldVision::Watched),
    );
    let dark = registry.register_field(
        "power",
        FieldDef::new(GROUND, FieldDecay::Instant, FieldVision::Dark),
    );

    assert_eq!(registry.field_def(watched).vision(), FieldVision::Watched);
    assert_eq!(registry.field_def(dark).vision(), FieldVision::Dark);
}

#[test]
#[should_panic(expected = "field 'creep' is already registered")]
fn register_rejects_duplicate_field() {
    let mut registry = utils::ground_registry();
    creep_field(&mut registry);
    creep_field(&mut registry);
}

#[test]
#[should_panic(expected = "field 'creep' covers unregistered layers")]
fn register_rejects_field_over_unregistered_layer() {
    utils::ground_registry().register_field(
        "creep",
        FieldDef::new(utils::WATER, FieldDecay::Instant, FieldVision::Dark),
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' projects an unregistered field")]
fn register_rejects_source_of_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry()
        .register(utils::standing("hive", GROUND).with_field_sources([emitter(creep)]));
}

#[test]
#[should_panic(expected = "entity type 'spore' reads an unregistered field for placement")]
fn register_rejects_placement_on_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register(
        utils::standing("spore", GROUND)
            .with_field_placement([FieldPlacement::Forbids { field: creep }]),
    );
}

#[test]
#[should_panic(expected = "entity type 'zergling' answers to an unregistered field")]
fn register_rejects_effect_of_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register(utils::standing("zergling", GROUND).with_field_effects([
        FieldEffect::new(
            creep,
            FieldAffiliation::Own,
            FieldSide::Outside,
            FieldEffectKind::Disabled,
        ),
    ]));
}

#[test]
#[should_panic(expected = "decay cycle must be positive")]
fn field_rejects_zero_decay_cycle() {
    FieldDef::new(GROUND, FieldDecay::Gradual { cycle: 0 }, FieldVision::Dark);
}

#[test]
#[should_panic(expected = "entity type 'hive' starts a field beyond its radius")]
fn register_rejects_source_starting_beyond_radius() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", GROUND).with_field_sources([FieldSourceDef::new(
            creep,
            3,
            FieldGrowth::Gradual {
                cycle: 2,
                initial_radius: 5,
            },
            None,
        )]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'hive' projects a field beyond its radius while constructing"
)]
fn register_rejects_source_constructing_beyond_radius() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", GROUND)
            .with_build_time(4)
            .with_field_sources([FieldSourceDef::new(creep, 3, FieldGrowth::Instant, Some(5))]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'hive' projects a field while constructing but is never constructed"
)]
fn register_rejects_source_constructing_on_type_never_built() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", GROUND).with_field_sources([FieldSourceDef::new(
            creep,
            3,
            FieldGrowth::Instant,
            Some(1),
        )]),
    );
}

#[test]
#[should_panic(expected = "entity type 'zergling' has a field effect with no modifiers")]
fn register_rejects_field_effect_with_no_modifiers() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", GROUND).with_field_effects([FieldEffect::new(
            creep,
            FieldAffiliation::Anyone,
            FieldSide::Inside,
            FieldEffectKind::Modifiers(Vec::new()),
        )]),
    );
}

#[test]
fn register_accepts_position_cast_with_field_effect() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register_skill(
        "spew",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Position,
                effect: EntityCastEffect::Field {
                    field: creep,
                    radius: 2,
                    action: FieldAction::Cover,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'zap' aims at a position but its effect needs an entity")]
fn register_rejects_position_cast_with_entity_effect() {
    utils::ground_registry().register_skill(
        "zap",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Position,
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'spew' acts on an unregistered field")]
fn register_rejects_cast_on_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register_skill(
        "spew",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Position,
                effect: EntityCastEffect::Field {
                    field: creep,
                    radius: 2,
                    action: FieldAction::Cover,
                },
            },
            requires: Vec::new(),
        },
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Registers `def` into a registry that already knows the "gold" resource kind.
fn gold_registry_with(def: EntityTypeDef) {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(def);
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

/// A free, timed, committed transition into `into` worn as `via` on the way.
fn morph_through(via: &str, into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        Some(via),
        MorphTime::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        Vec::new(),
        Vec::<String>::new(),
    )
}

/// A free, timed, committed transition into the named type.
fn morph_into(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        None,
        MorphTime::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        Vec::new(),
        Vec::<String>::new(),
    )
}

/// A gradually receding ground field named "creep", registered into `registry`.
fn creep_field(registry: &mut ContentRegistry) -> FieldId {
    registry.register_field(
        "creep",
        FieldDef::new(GROUND, FieldDecay::Gradual { cycle: 4 }, FieldVision::Dark),
    )
}

/// An instant emitter of `field` reaching three cells.
fn emitter(field: FieldId) -> FieldSourceDef {
    FieldSourceDef::new(field, 3, FieldGrowth::Instant, None)
}
