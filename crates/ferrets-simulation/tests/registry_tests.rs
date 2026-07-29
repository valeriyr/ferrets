//! Content validation at registration: [`ContentRegistry::register`] validates
//! each definition against the content already registered and panics on any
//! inconsistency, so a referenced type must be registered before the type that
//! references it.

use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::LayerId, nav_size::NavSize};
use ferrets_simulation::content::{
    skills::{SkillDef, SkillEffect, SkillTarget},
    stats::StatId,
    tags,
    {
        entity_type_def::EntityTypeDef,
        location::Solidity,
        registry::ContentRegistry,
        resource::{DepletionPolicy, HarvestData, HarvestVisibility},
    },
};

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
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_resource_carrier([("gold", HarvestData::new(5, 2, HarvestVisibility::Hidden))])
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
        worker()
            .with_resource_carrier([("wood", HarvestData::new(5, 2, HarvestVisibility::Visible))]),
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
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_train_time(4),
    );
    registry.register(
        EntityTypeDef::new("depot")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_build_time(6),
    );
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_trainer(["soldier"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["depot"]),
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
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
            .with_build_time(6)
            .with_trainer(["worker"]),
    );
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_train_time(4)
            .with_builder(["town_hall"]),
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
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
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
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("barracks")
            .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
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
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["nexus"]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "builds 'statue', which is not a registered constructible type")]
fn validate_rejects_unconstructible_built_type() {
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_builder(["statue"]),
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
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(2, Some("bones")),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("corpse")),
    );
}

#[test]
#[should_panic(expected = "entity type 'soldier' leaves an unregistered corpse type 'ghost'")]
fn register_rejects_unknown_corpse_type() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_dying(3, Some("ghost")),
    );
}

#[test]
#[should_panic(expected = "leaves a corpse type 'statue' that has no dying phase")]
fn register_rejects_corpse_without_dying_phase() {
    let mut registry = ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        GROUND,
        NavSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
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
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_attack(1, 1, 1, 2, 1)
            .with_dying(2, None),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
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
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
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
        NavSize::ONE,
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
        NavSize::ONE,
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

//
// ─── Stats ──────────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "has a non-positive max_health stat")]
fn register_rejects_non_positive_max_health() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_health(0),
    );
}

#[test]
#[should_panic(expected = "has a non-positive speed stat")]
fn register_rejects_non_positive_speed() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "has attack_range below its minimum of 1")]
fn register_rejects_zero_attack_range() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_attack(10, 0, 1, 2, 1),
    );
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_zero_attack_period() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_attack(10, 1, 1, 0, 0),
    );
}

#[test]
#[should_panic(expected = "has damage_point below its minimum of 1")]
fn register_rejects_zero_damage_point() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
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
            energy_cost: FixedU64::from_num(25),
            target: SkillTarget::Caster,
            effect: SkillEffect::Damage(FixedU64::from_num(5)),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
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
            energy_cost: FixedU64::ZERO,
            target: SkillTarget::Caster,
            effect: SkillEffect::Damage(FixedU64::from_num(5)),
        },
    );
    registry.register(
        EntityTypeDef::new("caster")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_health(20)
            .with_skills([shout]),
    );
    assert!(registry.entity("caster").is_some());
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_fractional_attack_period() {
    // Positive but below one whole tick: the engine reads the cycle as an integer,
    // so this would truncate to a phase the counter never reaches.
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_stat(StatId::ATTACK_PERIOD, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "has a damage_point beyond its attack_period")]
fn register_rejects_damage_point_beyond_attack_period() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_attack(10, 1, 1, 2, 5),
    );
}

#[test]
#[should_panic(expected = "carries the damage stat but is missing attack_range")]
fn register_rejects_attacker_without_weapon_stats() {
    let mut registry = ground_registry();
    registry.register(
        EntityTypeDef::new("worker")
            .with_location(GROUND, NavSize::ONE, Solidity::Solid)
            .with_stat(StatId::DAMAGE, FixedU64::from_num(5)),
    );
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
    EntityTypeDef::new("worker").with_location(GROUND, NavSize::ONE, Solidity::Solid)
}
