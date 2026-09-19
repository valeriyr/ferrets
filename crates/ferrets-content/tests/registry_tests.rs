//! Content validation at registration: [`ContentRegistry::register`] validates
//! each definition against the content already registered and panics on any
//! inconsistency, so a referenced type must be registered before the type that
//! references it.

mod utils;

use std::collections::BTreeSet;

use ferrets_content::{
    affiliation::Affiliation,
    annex::{AloneConduct, AnnexClaim, AnnexLife, AnnexWork},
    attack::{Delivery, Slain, Weapon},
    berths::BerthGroup,
    brood::OrphanFate,
    build::BuilderAttendance,
    costs::{self, Cost},
    dying::{Bequest, DeathKind, DyingDef, LeftBy},
    entity_buffs::EntityBuffDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        FieldAction, FieldCoverage, FieldDecay, FieldDef, FieldEffect, FieldEffectKind,
        FieldGrowth, FieldId, FieldPlacement, FieldSide, FieldSourceDef, FieldVision,
    },
    kinds::Kinds,
    location::Solidity,
    morph::{MorphCancel, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition},
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    player_stats::PlayerStatId,
    quantity::Quantity,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    requirement::Requirement,
    research::{ResearchDef, ResearcherDef},
    resource::{Banking, DepletionPolicy, HarvestData},
    skills::{
        Casting, EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach,
        SkillCaster, SkillDef,
    },
    stack_rule::StackRule,
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp},
    tags,
    transport::{PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
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
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    Banking::Carried,
                    Kinds::Any,
                ),
            )])
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
    gold_registry_with(utils::standing("worker", GROUND).with_resource_carrier([(
        "wood",
        HarvestData::new(
            5,
            5,
            2,
            WorkPresence::Present {
                crew: CrewLimit::ONE,
            },
            Banking::Carried,
            Kinds::Any,
        ),
    )]));
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
            .with_builder(
                ["depot"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            ),
    );

    registry.validate();
}

//
// ─── Berths and overbuilding ──────────────────────────────────────────────────
//

#[test]
fn validate_accepts_attachments_offered_by_their_jobs() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::sized("depot", GROUND, CellSize::new(2, 2))
            .with_build_time(6)
            .with_berths([(
                "rim",
                BerthGroup::new([point("0.5", "0.5"), point("1.5", "1.5")], 2),
            )]),
    );
    registry.register(
        utils::sized("vein", GROUND, CellSize::new(2, 2))
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_berths([("rim", BerthGroup::new([point("0.5", "0.5")], 1))]),
    );
    registry.register(
        utils::standing("sprite", GROUND)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_builder(
                ["depot"],
                BuilderAttendance::Crew(WorkPresence::Attached(Attachment::new(
                    "rim",
                    BerthStance::Still,
                ))),
            )
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Attached(Attachment::new(
                        "rim",
                        BerthStance::Roaming {
                            speed: FixedU64::lit("0.25"),
                            dwell: 2,
                        },
                    )),
                    Banking::Direct,
                    Kinds::Any,
                ),
            )]),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'sprite' attaches to berth group 'rim' of 'depot', which declares no such group"
)]
fn validate_rejects_builder_attaching_to_group_its_site_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(utils::sized("depot", GROUND, CellSize::new(2, 2)).with_build_time(6));
    registry.register(
        utils::standing("sprite", GROUND)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["depot"],
                BuilderAttendance::Crew(WorkPresence::Attached(Attachment::new(
                    "rim",
                    BerthStance::Still,
                ))),
            ),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'sprite' attaches to berth group 'canopy' of gold sources, and no gold source it may work declares such a group"
)]
fn validate_rejects_carrier_attaching_to_group_no_source_offers() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("sprite", GROUND)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Attached(Attachment::new("canopy", BerthStance::Still)),
                    Banking::Direct,
                    Kinds::Any,
                ),
            )]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_overbuilding_of_matching_source() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::sized("mine", GROUND, CellSize::new(2, 2))
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::sized("shaft_house", GROUND, CellSize::new(2, 2))
            .with_build_time(6)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' overbuilds 'mine' on a footprint of another size"
)]
fn validate_rejects_overbuilding_source_of_another_size() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::sized("shaft_house", GROUND, CellSize::new(2, 2))
            .with_build_time(6)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' overbuilds 'mine' but is not a resource source itself"
)]
fn validate_rejects_overbuilding_by_type_that_is_not_source_itself() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", GROUND)
            .with_build_time(6)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'shaft_house' overbuilds 'seam', which is not registered")]
fn validate_rejects_overbuilding_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("shaft_house", GROUND)
            .with_build_time(6)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("seam"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' overbuilds 'boulder', which is not a resource source"
)]
fn validate_rejects_overbuilding_type_that_is_no_source() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(utils::standing("boulder", GROUND));
    registry.register(
        utils::standing("shaft_house", GROUND)
            .with_build_time(6)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("boulder"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' yields wood but overbuilds 'mine', which yields gold"
)]
fn validate_rejects_overbuilding_source_of_another_kind() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register_resource("wood");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", GROUND)
            .with_build_time(6)
            .with_resource_source("wood", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' overbuilds 'mine', which occupies other layers"
)]
fn validate_rejects_overbuilding_source_on_other_layers() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    let air = registry.register_layer("air");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", air)
            .with_build_time(6)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'shaft_house' overbuilds 'mine' but is not constructible")]
fn validate_rejects_overbuilding_by_type_nothing_builds() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("mine", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", GROUND)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
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
            .with_builder(
                ["town_hall"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            ),
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
            .with_builder(
                ["nexus"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            ),
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
            .with_builder(
                ["statue"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            ),
    );
    registry.validate();
}

//
// ─── What a death leaves ──────────────────────────────────────────────────────
//

#[test]
fn register_accepts_terminating_decay_chains() {
    let mut registry = utils::ground_registry();

    registry.register(remains("bones", 200));
    registry.register(remains("corpse", 600).with_leaves(utils::leaves("bones")));
    registry.register(utils::standing("soldier", GROUND).with_dying(3, utils::leaves("corpse")));
    registry.validate();
}

#[test]
fn register_accepts_death_that_leaves_standing_units() {
    let mut registry = utils::ground_registry();

    registry.register(utils::standing("broodling", GROUND));
    registry.register(utils::standing("hive", GROUND).with_dying(
        2,
        [Bequest::new(
            "broodling",
            2,
            LeftBy::Named(vec![DeathKind::Killed]),
        )],
    ));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'soldier' leaves 'ghost', which is not registered")]
fn validate_rejects_leaving_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("soldier", GROUND).with_dying(3, utils::leaves("ghost")));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'bones' is remains, but defines live-gameplay data that remains never use"
)]
fn register_rejects_remains_with_live_gameplay_data() {
    let mut registry = utils::ground_registry();
    registry.register(remains("bones", 200).with_health(10).with_attack(
        utils::weapon(GROUND),
        1,
        1,
        1,
        2,
        1,
    ));
}

#[test]
#[should_panic(expected = "entity type 'bones' is remains but carries no lifetime")]
fn register_rejects_remains_that_never_go_away() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("bones", GROUND).with_tags(["remains"]));
}

#[test]
#[should_panic(expected = "entity type 'corpse' is remains and states a dying time")]
fn register_rejects_remains_waiting_to_leave() {
    let mut registry = utils::ground_registry();

    // A body has lain its whole life already: what it rots into goes down the
    // tick its decay ends, and a wait on top of that is a number nothing
    // reads.
    registry.register(remains("corpse", 600).with_dying(2, []));
}

#[test]
#[should_panic(expected = "a dying time of no ticks is no dying time at all")]
fn dying_of_no_ticks_is_refused() {
    DyingDef::new(Some(0), []);
}

#[test]
#[should_panic(expected = "a death that waits for nothing and leaves nothing is no dying at all")]
fn dying_that_waits_for_nothing_and_leaves_nothing_is_refused() {
    DyingDef::new(None, []);
}

#[test]
#[should_panic(expected = "entity type 'soldier' morphing into 'corpse' names remains")]
fn validate_rejects_transition_into_remains() {
    let mut registry = utils::ground_registry();

    // Only a death leaves a body: a unit that could simply turn into one
    // would lay remains nothing died for.
    registry.register(remains("corpse", 600));
    registry.register(utils::standing("soldier", GROUND).with_morphs([morph_into("corpse")]));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'larva' morphing into 'hatchling' wears remains")]
fn validate_rejects_transition_wearing_remains() {
    let mut registry = utils::ground_registry();

    registry.register(remains("corpse", 600));
    registry.register(utils::standing("hatchling", GROUND).with_health(30));
    registry.register(
        utils::standing("larva", GROUND)
            .with_health(20)
            .with_morphs([morph_through("corpse", "hatchling")]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "rots into a chain that comes back round to 'corpse'")]
fn validate_rejects_decay_chain_that_comes_back_round() {
    let mut registry = utils::ground_registry();

    // Bodies that rot into each other are bodies that never leave the map.
    registry.register(remains("corpse", 600).with_leaves(utils::leaves("bones")));
    registry.register(remains("bones", 200).with_leaves(utils::leaves("corpse")));
    registry.validate();
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
            .with_attack_def(GROUND, Delivery::Instant, None, Slain::Remains),
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
    registry.register(utils::standing("worker", GROUND).with_builder(
        ["depot"],
        BuilderAttendance::Crew(WorkPresence::Hidden {
            crew: CrewLimit::ONE,
        }),
    ));
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
    gold_registry_with(utils::standing("worker", GROUND).with_resource_carrier([(
        "gold",
        HarvestData::new(
            5,
            5,
            2,
            WorkPresence::Present {
                crew: CrewLimit::ONE,
            },
            Banking::Carried,
            Kinds::Any,
        ),
    )]));
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
            .with_attack_def(GROUND, Delivery::Instant, None, Slain::Remains),
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
                Kinds::tags(["infantry"]),
                Affiliation::Own,
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
                Kinds::tags(["infantry"]),
                Affiliation::Own,
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
                Kinds::tags(["infantry"]),
                Affiliation::Own,
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
                Kinds::tags(["infantry"]),
                Affiliation::Own,
                PassengerFate::Destroy,
                PassengerConduct::Shelter,
            ),
    );
}

#[test]
#[should_panic(expected = "entity type 'wagon' carries tag 'critters', which is not registered")]
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
                Kinds::tags(["critters"]),
                Affiliation::Own,
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
                Kinds::types(["footman"]),
                Affiliation::Own,
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
#[should_panic(expected = "entity type 'priest' has ritual_time below its minimum of 1")]
fn register_rejects_content_stat_below_its_floor() {
    let mut registry = utils::ground_registry();
    let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ONE);

    // The fold would raise it on the first tick, so the authored number is one
    // the type never actually has.
    registry.register(
        utils::standing("priest", GROUND)
            .with_health(20)
            .with_stat(ritual_time, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "entity stat 'ritual_time' is already registered")]
fn register_rejects_stat_declared_twice() {
    let mut registry = ContentRegistry::default();
    registry.register_entity_stat("ritual_time", FixedU64::ONE);

    // A second declaration would leave the floor depending on which of them
    // ran first, as every other name the registry mints is declared once.
    registry.register_entity_stat("ritual_time", FixedU64::ZERO);
}

#[test]
#[should_panic(expected = "'max_supply' is already registered as a player stat")]
fn register_stat_rejects_player_stat_name() {
    ContentRegistry::default().register_entity_stat("max_supply", FixedU64::ZERO);
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                Kinds::tags(["building"]),
                RepairRate::Production,
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
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
                Kinds::tags(["building"]),
                RepairRate::Production,
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
                false,
                RepairCost::Free,
                None,
            ),
    );
}

#[test]
fn validate_accepts_repairer_mending_its_own_kind() {
    let mut registry = utils::ground_registry();

    // A filter is settled once everything is registered, so a mechanic may
    // mend mechanics — and anything declared after it.
    registry.register(
        utils::standing("mechanic", GROUND)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                Kinds::types(["mechanic", "tank"]),
                RepairRate::Production,
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
                false,
                RepairCost::Free,
                None,
            ),
    );
    registry.register(utils::standing("tank", GROUND).with_health(40));

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'worker' repairs tag 'mechanical', which is not registered")]
fn validate_rejects_repairer_mending_unknown_tag() {
    // "building" is pre-registered, so an unknown tag has to be one content would
    // have had to declare itself.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", GROUND)
            .with_health(20)
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                Kinds::tags(["mechanical"]),
                RepairRate::Production,
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
                false,
                RepairCost::Free,
                None,
            ),
    );
    registry.validate();
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
                Kinds::tags(["building"]),
                RepairRate::Production,
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
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
                Kinds::tags(["biological"]),
                RepairRate::PerTick(FixedU64::ONE),
                WorkPresence::Present {
                    crew: CrewLimit::ONE,
                },
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
        Kinds::tags(["biological"]),
        RepairRate::PerTick(FixedU64::ZERO),
        WorkPresence::Present {
            crew: CrewLimit::ONE,
        },
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
        ResearchDef::new(costs::cost([("gold", 30)]), 10, Some(buff), Vec::new()),
    );
    let tactics = registry.register_research(
        "tactics",
        ResearchDef::new(Cost::new(), 5, None, [Requirement::Research(smithing)]),
    );

    assert!(registry.has_research("smithing"));
    assert_eq!(registry.research("smithing"), Some(smithing));
    assert_eq!(registry.research_name(tactics), Some("tactics"));
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
    assert_eq!(registry.research_def(tactics).unwrap().buff, None);

    // Re-registering a name keeps the first definition and returns its id.
    let again = registry.register_research(
        "smithing",
        ResearchDef::new(Cost::new(), 99, None, Vec::new()),
    );
    assert_eq!(again, smithing);
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
}

#[test]
#[should_panic(expected = "research name must not be empty")]
fn register_research_rejects_empty_name() {
    utils::ground_registry()
        .register_research("", ResearchDef::new(Cost::new(), 10, None, Vec::new()));
}

#[test]
#[should_panic(expected = "research 'smithing' costs unregistered resource kind 'gold'")]
fn register_research_rejects_unknown_cost_kind() {
    utils::ground_registry().register_research(
        "smithing",
        ResearchDef::new(costs::cost([("gold", 30)]), 10, None, Vec::new()),
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
        ResearchDef::new(Cost::new(), 10, Some(buff), Vec::new()),
    );
}

#[test]
#[should_panic(expected = "research_time must be greater than 0")]
fn research_def_rejects_zero_time() {
    ResearchDef::new(Cost::new(), 0, None, Vec::new());
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
    let research = foreign.register_research(
        "smithing",
        ResearchDef::new(Cost::new(), 10, None, Vec::new()),
    );
    utils::ground_registry().register(utils::standing("lab", GROUND).with_researcher([research]));
}

#[test]
#[should_panic(
    expected = "entity type 'keep' offers docks but raises its sites as a hidden builder"
)]
fn validate_rejects_primary_that_goes_inside_its_annex_site() {
    let mut registry = utils::ground_registry();
    registry.register(
        EntityTypeDef::new("keep")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::types(["lookout"]))]),
    );
    registry.register(
        EntityTypeDef::new("lookout")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_build_time(4)
            .with_annex(
                AloneConduct::Standing {
                    work: AnnexWork::Works,
                    life: AnnexLife::Endures,
                },
                AnnexClaim::Bound,
            ),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'mast' fades but does not carry the health_drain stat")]
fn validate_rejects_fading_annex_without_drain_stat() {
    let mut registry = utils::ground_registry();
    registry.register_tag("building");
    registry.register(
        EntityTypeDef::new("keep")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["mast"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::types(["mast"]))]),
    );
    registry.register(
        EntityTypeDef::new("mast")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_build_time(4)
            .with_annex(
                AloneConduct::Standing {
                    work: AnnexWork::Works,
                    life: AnnexLife::Fades {
                        per_tick: FixedU64::from_num(2),
                    },
                },
                AnnexClaim::Bound,
            ),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'miner' declares a crew limit of nobody")]
fn validate_rejects_crew_limit_of_nobody() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("seam", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", GROUND)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    // Written past the constructor that would have refused it.
                    WorkPresence::Hidden {
                        crew: CrewLimit::Limit(0),
                    },
                    Banking::Carried,
                    Kinds::Any,
                ),
            )]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'miner' harvests gold from nothing at all")]
fn validate_rejects_harvest_source_list_that_names_nobody() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("seam", GROUND).with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", GROUND)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    Banking::Carried,
                    // Written past the constructor that would have refused it.
                    Kinds::Only(BTreeSet::new()),
                ),
            )]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' docks entity type 'ghost', which is not registered")]
fn validate_rejects_dock_taking_unregistered_type() {
    let mut registry = utils::ground_registry();
    // The builder catalogue is sound, so `validate_builds` has nothing to say
    // and the dock's own unregistered name is what is left to catch.
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::types(["ghost"]))]),
    );
    registry.register(annex("lookout", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' carries the speed stat")]
fn validate_rejects_annex_that_moves() {
    let mut registry = utils::ground_registry();
    registry.register(primary(["lookout"]));
    registry.register(annex("lookout", CellSize::ONE).with_movement(
        FixedU64::from_num(0.5),
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'miner' harvests gold from entity type 'refinery', which is not registered"
)]
fn validate_rejects_harvest_source_that_is_not_registered() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("miner", GROUND)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    Banking::Carried,
                    Kinds::types(["refinery"]),
                ),
            )]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'miner' harvests gold from 'grove', which is no gold source"
)]
fn validate_rejects_harvest_source_of_another_kind() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register_resource("wood");
    registry.register(
        utils::standing("grove", GROUND).with_resource_source("wood", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", GROUND)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "gold",
                HarvestData::new(
                    5,
                    5,
                    2,
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    Banking::Carried,
                    Kinds::types(["grove"]),
                ),
            )]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_dock_that_takes_any_annex() {
    let mut registry = utils::ground_registry();
    // A filter that names nobody in particular takes whatever is an annex,
    // and what it sweeps in besides goes on standing where it stands.
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lab"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::Any)]),
    );
    registry.register(annex("lab", CellSize::ONE));
    registry.register(utils::standing("runner", GROUND).with_health(10));

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' offers a dock at (2, 0) that names no annex")]
fn validate_rejects_dock_that_names_no_annex() {
    let mut registry = utils::ground_registry();
    registry.register_tag("workshop");
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lab"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::tags(["workshop"]))]),
    );
    registry.register(annex("lab", CellSize::ONE));

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' docks 'runner', which is not an annex")]
fn validate_rejects_dock_that_takes_type_that_is_no_annex() {
    let mut registry = utils::ground_registry();
    registry.register(primary(["runner"]));
    registry.register(
        utils::standing("runner", GROUND)
            .with_health(10)
            .with_build_time(4),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' offers a dock at (0, 0), inside its own footprint")]
fn validate_rejects_dock_inside_its_primarys_own_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(0, 0), Kinds::types(["lookout"]))]),
    );
    registry.register(annex("lookout", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' docks 'lookout' but cannot raise it")]
fn validate_rejects_dock_whose_primary_cannot_raise_what_it_takes() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_docks([(CellPos::new(2, 0), Kinds::types(["lookout"]))]),
    );
    registry.register(annex("lookout", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' offers two docks at (2, 0)")]
fn validate_rejects_two_docks_on_one_cell() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([
                (CellPos::new(2, 0), Kinds::types(["lookout"])),
                (CellPos::new(2, 0), Kinds::types(["lookout"])),
            ]),
    );
    registry.register(annex("lookout", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'keep' docks 'beacon' at (3, 0), over the ground 'lookout' stands on"
)]
fn validate_rejects_docks_whose_annexes_would_stand_on_each_other() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("keep", GROUND, CellSize::new(2, 2))
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout", "beacon"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            // A two-cell lookout at (2, 0) covers (3, 0) too, which is where
            // the beacon is told to stand.
            .with_docks([
                (CellPos::new(2, 0), Kinds::types(["lookout"])),
                (CellPos::new(3, 0), Kinds::types(["beacon"])),
            ]),
    );
    registry.register(annex("lookout", CellSize::new(2, 1)));
    registry.register(annex("beacon", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' does not claim the cells it stands on")]
fn validate_rejects_annex_that_does_not_claim_its_cells() {
    let mut registry = utils::ground_registry();
    registry.register(primary(["lookout"]));
    registry.register(
        EntityTypeDef::new("lookout")
            .with_location(GROUND, CellSize::ONE, Solidity::Passable)
            .with_health(10)
            .with_build_time(4)
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' has no health pool")]
fn validate_rejects_annex_without_health() {
    let mut registry = utils::ground_registry();
    registry.register(primary(["lookout"]));
    registry.register(
        utils::standing("lookout", GROUND)
            .with_build_time(4)
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' is not constructible")]
fn validate_rejects_annex_that_cannot_be_built() {
    let mut registry = utils::ground_registry();
    // No primary: one that raises it would have to declare it constructible,
    // and registered ahead of the annex it would be `validate_builds` that
    // spoke first.
    registry.register(
        utils::standing("lookout", GROUND)
            .with_health(10)
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' fits no registered dock")]
fn validate_rejects_annex_no_dock_takes() {
    let mut registry = utils::ground_registry();
    registry.register(annex("lookout", CellSize::ONE));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'walking_keep' offers docks but carries the speed stat")]
fn validate_rejects_docks_on_type_that_moves() {
    let mut registry = utils::ground_registry();
    registry.register(
        EntityTypeDef::new("walking_keep")
            .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_health(100)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::types(["lookout"]))]),
    );
    registry.register(
        EntityTypeDef::new("lookout")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(10)
            .with_build_time(4)
            .with_annex(
                AloneConduct::Standing {
                    work: AnnexWork::Works,
                    life: AnnexLife::Endures,
                },
                AnnexClaim::Bound,
            ),
    );
    registry.validate();
}

//
// ─── Requirements ─────────────────────────────────────────────────────────────
//

// A requirement names its own kind. An entity or annex name is a forward
// reference, checked by `validate()` against the complete registry; a research
// is a handle the content resolved when it was read.

#[test]
fn validate_accepts_type_tag_and_research_requirements() {
    let mut registry = utils::ground_registry();
    let smithing = registry.register_research(
        "smithing",
        ResearchDef::new(
            Cost::new(),
            10,
            None,
            [Requirement::EntityType("lab".to_string())],
        ),
    );
    // The knight's requirements name a type registered after it, the reserved
    // "building" tag, and a research.
    registry.register(utils::standing("knight", GROUND).with_requires([
        Requirement::EntityType("lab".to_string()),
        Requirement::Tag(tags::BUILDING.to_string()),
        Requirement::Research(smithing),
    ]));
    registry.register(utils::standing("lab", GROUND).with_researcher([smithing]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires the entity type 'chapel', which is not registered"
)]
fn validate_rejects_unknown_requirement() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", GROUND)
            .with_requires([Requirement::EntityType("chapel".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires the tag 'forge', which is also a registered entity type"
)]
fn validate_rejects_ambiguous_requirement() {
    let mut registry = utils::ground_registry();
    registry.register_tag("forge");
    registry.register(EntityTypeDef::new("forge").with_location(
        GROUND,
        CellSize::ONE,
        Solidity::Solid,
    ));
    registry.register(
        utils::standing("knight", GROUND).with_requires([Requirement::Tag("forge".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "research 'smithing' requires the entity type 'chapel', which is not registered"
)]
fn validate_rejects_unknown_research_requirement() {
    let mut registry = utils::ground_registry();
    registry.register_research(
        "smithing",
        ResearchDef::new(
            Cost::new(),
            10,
            None,
            [Requirement::EntityType("chapel".to_string())],
        ),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "skill 'war_cry' requires the entity type 'chapel', which is not registered"
)]
fn validate_rejects_unknown_skill_requirement() {
    let mut registry = utils::ground_registry();
    let haste = haste_buff(&mut registry);
    let mut skill = player_cast(haste);
    skill.requires = vec![Requirement::EntityType("chapel".to_string())];
    registry.register_skill("war_cry", skill);
    registry.validate();
}

#[test]
fn validate_accepts_research_requirement_on_skill() {
    let mut registry = utils::ground_registry();
    let haste = haste_buff(&mut registry);
    let war_drums = registry.register_research(
        "war_drums",
        ResearchDef::new(Cost::new(), 10, None, Vec::new()),
    );
    let mut skill = player_cast(haste);
    skill.requires = vec![Requirement::Research(war_drums)];
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
fn validate_accepts_transition_with_odd_footprint_difference() {
    // 1x1 -> 2x2 recentres by half a cell; the landing settles the anchor
    // onto the lattice where the model needs it, so nothing is refused here.
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
    expected = "entity type 'walker' morphing into 'flier' requires the entity type \
                'jet_pack', which is not registered"
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
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphInterrupted::Reverts,
                MorphReason::Change,
                Vec::new(),
                [Requirement::EntityType("jet_pack".to_string())],
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
    let stat = registry.register_entity_stat("change_time", FixedU64::ONE);
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
                Quantity::Stat(stat),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphInterrupted::Reverts,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
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
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphInterrupted::Reverts,
                MorphReason::Change,
                vec![EntityCastCost::Energy(FixedU64::from_num(20))],
                Vec::new(),
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
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphInterrupted::Reverts,
                MorphReason::Change,
                vec![EntityCastCost::Resources(costs::cost([("gold", 50)]))],
                Vec::new(),
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
                Quantity::Constant(20),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                MorphInterrupted::Reverts,
                MorphReason::Change,
                vec![
                    EntityCastCost::Resources(costs::cost([("gold", 50)])),
                    EntityCastCost::Energy(FixedU64::from_num(20)),
                ],
                Vec::new(),
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
            Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
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
            Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
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
            Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
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
            .with_health(300)
            .with_field_sources([emitter(creep)])
            .with_field_placement([FieldPlacement::Requires {
                field: creep,
                of: Affiliation::Anyone,
                coverage: FieldCoverage::Footprint,
            }])
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldSide::Inside,
                FieldEffectKind::Modifiers(vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
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
            Affiliation::Own,
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
            Affiliation::Anyone,
            FieldSide::Inside,
            FieldEffectKind::Modifiers(Vec::new()),
        )]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'zergling' has a field effect on stat 'health_drain', which the type does not carry"
)]
fn register_rejects_field_effect_on_stat_type_lacks() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", GROUND)
            .with_health(35)
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldSide::Outside,
                FieldEffectKind::Modifiers(vec![EntityModifier {
                    stat: EntityStatId::HEALTH_DRAIN,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(1),
                }]),
            )]),
    );
}

#[test]
#[should_panic(expected = "skill 'scan' watches for no time at all")]
fn register_rejects_watch_that_lasts_no_time() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "scan",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Position,
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Watch {
                    radius: 3,
                    duration: 0,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "player-cast skill 'haste' requires something of an acting entity")]
fn register_rejects_player_cast_asking_something_of_actor() {
    let mut registry = utils::ground_registry();
    let buff = haste_buff(&mut registry);
    let mut skill = player_cast(buff);
    // No entity casts it, so there is nothing for a docked annex to be asked
    // of — and the scope is read off the requirement itself, so the name it
    // carries never has to resolve.
    skill.requires = vec![Requirement::Annexed("lookout".to_string())];
    registry.register_skill("haste", skill);
}

#[test]
#[should_panic(
    expected = "entity type 'marine' requires 'runner' docked, which is not a registered annex"
)]
fn validate_rejects_annexed_requirement_naming_type_that_is_no_annex() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("runner", GROUND).with_health(10));
    registry.register(
        utils::standing("marine", GROUND)
            .with_health(10)
            .with_requires([Requirement::Annexed("runner".to_string())]),
    );
    registry.validate();
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
#[should_panic(expected = "skill 'raise_dead' summons 'corpse', which is remains")]
fn register_rejects_summon_of_remains() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("corpse", GROUND)
            .with_tags(["remains"])
            .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
    );
    let corpse = registry.type_id("corpse").expect("just registered");
    registry.register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    entity_type: corpse,
                    count: 2,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_remains_cast_with_summon_effect() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("skeleton", GROUND));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    registry.register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    entity_type: skeleton,
                    count: 2,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'smite' aims at entity type 'ghost', which is not registered")]
fn validate_rejects_skill_aimed_at_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "smite",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::types(["ghost"]),
                },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );

    registry.validate();
}

#[test]
fn validate_accepts_skill_aimed_at_type_that_carries_it() {
    let mut registry = utils::ground_registry();

    // A skill is registered before the type that carries it, so an aim is
    // settled once everything is registered: a priest may heal priests.
    let heal = registry.register_skill(
        "heal",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Allied,
                    kinds: Kinds::types(["priest"]),
                },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Heal(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("priest", GROUND)
            .with_health(20)
            .with_skills([heal]),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "skill 'feast' aims at the fallen, which only a summon may spend")]
fn register_rejects_remains_cast_with_other_effect() {
    utils::ground_registry().register_skill(
        "feast",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Heal(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_summon_aimed_at_caster_and_at_cell() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("skeleton", GROUND));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    let summons = |target| SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            target,
            reach: Reach::Wherever,
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                entity_type: skeleton,
                count: 1,
            },
        },
        requires: Vec::new(),
    };
    registry.register_skill("guard_me", summons(EntityCastTarget::Caster));
    registry.register_skill("guard_there", summons(EntityCastTarget::Position));
}

#[test]
#[should_panic(expected = "skill 'raise_dead' summons an unregistered entity type")]
fn register_rejects_summon_of_foreign_type() {
    let mut foreign = utils::ground_registry();
    foreign.register(utils::standing("skeleton", GROUND));
    let skeleton = foreign.type_id("skeleton").expect("just registered");
    utils::ground_registry().register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    entity_type: skeleton,
                    count: 1,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'raise_dead' summons nothing at all")]
fn register_rejects_summon_of_nobody() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("skeleton", GROUND));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    registry.register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    entity_type: skeleton,
                    count: 0,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_reach_read_from_stat_caster_carries() {
    let mut registry = utils::ground_registry();
    let raise_range = registry.register_entity_stat("raise_range", FixedU64::ONE);
    let bolt = registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::Any,
                },
                reach: Reach::Within(Quantity::Stat(raise_range)),
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("necromancer", GROUND)
            .with_stat(raise_range, FixedU64::from_num(6))
            .with_skills([bolt]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'necromancer' has skill 'bolt' reading its reach from a stat it does not carry"
)]
fn register_rejects_reach_by_stat_caster_lacks() {
    let mut registry = utils::ground_registry();
    let raise_range = registry.register_entity_stat("raise_range", FixedU64::ONE);
    let bolt = registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::Any,
                },
                reach: Reach::Within(Quantity::Stat(raise_range)),
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    registry.register(utils::standing("necromancer", GROUND).with_skills([bolt]));
}

#[test]
#[should_panic(
    expected = "entity type 'necromancer' has skill 'bolt' reading its cast point from a stat it does not carry"
)]
fn register_rejects_cast_point_by_stat_caster_lacks() {
    let mut registry = utils::ground_registry();
    let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ONE);
    let bolt = registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::Any,
                },
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Constant(20),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    registry.register(utils::standing("necromancer", GROUND).with_skills([bolt]));
}

#[test]
#[should_panic(
    expected = "skill 'bolt' is cast over no time at all: an instant cast declares no `cast`"
)]
fn register_rejects_cast_over_no_time() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Constant(0),
                    period: Quantity::Constant(10),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'bolt' frees its caster before the cast lands")]
fn register_rejects_cast_period_stat_that_folds_under_its_point() {
    let mut registry = utils::ground_registry();
    // The least the period ever folds to is its floor, so a stat floored
    // below the point frees the caster before the cast lands.
    let recovery = registry.register_entity_stat("recovery", FixedU64::ONE);
    registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Constant(10),
                    period: Quantity::Stat(recovery),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'bolt' frees its caster before the cast lands: 5 < 10")]
fn register_rejects_cast_period_shorter_than_its_point() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Constant(10),
                    period: Quantity::Constant(5),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "skill 'bolt' is cast over no time at all")]
fn register_rejects_cast_point_from_floorless_stat() {
    let mut registry = utils::ground_registry();
    let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ZERO);
    registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Stat(ritual_time),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_cast_worked_on_stats_caster_carries() {
    let mut registry = utils::ground_registry();
    let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ONE);
    let bolt = registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                target: EntityCastTarget::Standing {
                    side: Affiliation::Enemy,
                    kinds: Kinds::Any,
                },
                reach: Reach::Wherever,
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Stat(ritual_time),
                },
                effect: EntityCastEffect::Damage(FixedU64::ONE),
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("necromancer", GROUND)
            .with_stat(ritual_time, FixedU64::from_num(20))
            .with_skills([bolt]),
    );
}

#[test]
fn register_accepts_weapon_beside_carrier() {
    let mut registry = utils::ground_registry();
    registry.register_resource("wood");
    registry.register(
        utils::standing("tree", GROUND).with_resource_source("wood", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("ghoul", GROUND)
            .with_attack_def(GROUND, Delivery::Instant, None, Slain::Remains)
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(5))
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(10))
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(5))
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_carrier([(
                "wood".to_string(),
                HarvestData::new(
                    20,
                    20,
                    1,
                    WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    },
                    Banking::Carried,
                    Kinds::Any,
                ),
            )]),
    );
    registry.validate();
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
                reach: Reach::Wherever,
                casting: Casting::Instant,
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
// ─── Breeder and broodling ───────────────────────────────────────────────────
//

#[test]
fn validate_accepts_breeder_and_berthed_broodling() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 3, 2));
    registry.register(grub("grub"));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' names a type that is not registered"
)]
fn validate_rejects_brood_of_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 3, 2));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'hatch' breeding 'grub' names a type that is no broodling")]
fn validate_rejects_brood_of_type_that_is_no_broodling() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 3, 2));
    registry.register(utils::standing("grub", GROUND).with_health(25));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' seats it in berth group 'brood', which the breeder does not declare"
)]
fn validate_rejects_berthed_broodling_in_group_breeder_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("hatch", GROUND, CellSize::new(3, 3))
            .with_health(300)
            .with_breeder("grub", Quantity::Constant(10), 2, 0, OrphanFate::Perish),
    );
    registry.register(grub("grub"));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' keeps up to 3 but berth group 'brood' seats only 2"
)]
fn validate_rejects_brood_limit_above_slots() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 2, 3));
    registry.register(grub("grub"));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'hatch' breeding 'hatch' breeds its own type")]
fn validate_rejects_breeder_breeding_its_own_type() {
    let mut registry = utils::ground_registry();
    registry.register(
        hatch("hatch", 3, 2)
            .with_breeder("hatch", Quantity::Constant(10), 2, 0, OrphanFate::Perish)
            .with_broodling(Attachment::new("brood", BerthStance::Still)),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' seats a type wider than one cell in a berth"
)]
fn validate_rejects_berthed_broodling_wider_than_one_cell() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 3, 2));
    registry.register(
        utils::sized("grub", GROUND, CellSize::new(2, 2))
            .with_health(25)
            .with_broodling(Attachment::new("brood", BerthStance::Still)),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'hatch' breeding 'grub' names a type that can move")]
fn validate_rejects_bred_type_that_can_move() {
    let mut registry = utils::ground_registry();
    registry.register(hatch("hatch", 3, 2));
    registry.register(grub("grub").with_movement(
        FixedU64::from_num(0.5),
        FixedU64::from_num(0.5),
        FixedU64::ONE,
        FixedU64::from_num(360),
        FixedU64::from_num(360),
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' reads its period from a stat the type does not carry"
)]
fn validate_rejects_brood_period_from_stat_breeder_lacks() {
    let mut registry = utils::ground_registry();
    let brood_period = registry.register_entity_stat("brood_period", FixedU64::ONE);
    registry.register(hatch("hatch", 3, 2).with_breeder(
        "grub",
        Quantity::Stat(brood_period),
        2,
        0,
        OrphanFate::Perish,
    ));
    registry.register(grub("grub"));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'grub' is a broodling, and no registered type breeds it")]
fn validate_rejects_broodling_nobody_breeds() {
    let mut registry = utils::ground_registry();
    registry.register(grub("grub"));
    registry.validate();
}

#[test]
fn validate_accepts_interim_form_of_other_solidity() {
    let mut registry = utils::ground_registry();
    registry.register(
        EntityTypeDef::new("grub")
            .with_location(GROUND, CellSize::ONE, Solidity::Passable)
            .with_health(25)
            .with_morphs([morph_through("egg", "worker")]),
    );
    registry.register(utils::standing("egg", GROUND).with_health(100));
    registry.register(utils::standing("worker", GROUND).with_health(30));
    registry.validate();
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A type that lies where it is left and is gone after `decay` ticks.
fn remains(name: &str, decay: u32) -> EntityTypeDef {
    utils::standing(name, GROUND)
        .with_tags(["remains"])
        .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(decay))
}

/// A 2x2 primary that raises the `annexes` it docks at (2, 0), standing on the
/// site to do it.
fn primary(annexes: [&str; 1]) -> EntityTypeDef {
    utils::sized("keep", GROUND, CellSize::new(2, 2))
        .with_health(100)
        .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
        .with_builder(
            annexes,
            BuilderAttendance::Crew(WorkPresence::Present {
                crew: CrewLimit::ONE,
            }),
        )
        .with_docks([(CellPos::new(2, 0), Kinds::types(annexes))])
}

/// A constructible annex of `size` that endures alone and is bound to its owner.
fn annex(name: &str, size: CellSize) -> EntityTypeDef {
    utils::sized(name, GROUND, size)
        .with_health(10)
        .with_build_time(4)
        .with_annex(standing_annex(), AnnexClaim::Bound)
}

/// Standing alone, working, and not fading.
fn standing_annex() -> AloneConduct {
    AloneConduct::Standing {
        work: AnnexWork::Works,
        life: AnnexLife::Endures,
    }
}

/// A berth point from decimal strings, in cells from the footprint's anchor.
fn point(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(
        x.parse().expect("a decimal string"),
        y.parse().expect("a decimal string"),
    )
}

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

/// A 3×3 breeder of `grub`s with a brood group of `slots` seats and a limit of
/// `limit`.
fn hatch(name: &str, slots: usize, limit: usize) -> EntityTypeDef {
    utils::sized(name, GROUND, CellSize::new(3, 3))
        .with_health(300)
        .with_berths([(
            "brood",
            BerthGroup::new(
                [
                    point("0.5", "3.5"),
                    point("1.5", "3.5"),
                    point("2.5", "3.5"),
                ],
                slots,
            ),
        )])
        .with_breeder("grub", Quantity::Constant(10), limit, 0, OrphanFate::Perish)
}

/// A passable one-cell broodling that sits still in the `brood` group.
fn grub(name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND, CellSize::ONE, Solidity::Passable)
        .with_health(25)
        .with_broodling(Attachment::new("brood", BerthStance::Still))
}

/// A free, timed, committed transition into `into` worn as `via` on the way.
fn morph_through(via: &str, into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        Some(via),
        Quantity::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        MorphInterrupted::Reverts,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
    )
}

/// A free, timed, committed transition into the named type.
fn morph_into(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        None,
        Quantity::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        MorphInterrupted::Reverts,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
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
