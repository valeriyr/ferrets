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
    concealment::Concealment,
    cost::Cost,
    detection::Detection,
    domain::{DomainDef, DomainId},
    dying::{Bequest, DeathKind, DyingDef, LeftBy},
    entity_buffs::{EntityBuffDef, EntityBuffId, Interruption, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::{
        Emission, FieldAction, FieldCoverage, FieldDecay, FieldDef, FieldEffect, FieldGrowth,
        FieldId, FieldPlacement, FieldSourceDef, FieldVision,
    },
    gateway::{Doors, Joins},
    kinds::{Kind, Kinds},
    landing::Landing,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    network::NetworkDef,
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    player_stats::PlayerStatId,
    pool::{Pool, PoolInitial},
    pool_def::{PoolId, PoolRole},
    pool_shift::PoolShift,
    price::{self, Price},
    quantity::Quantity,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    requirement::{Bound, Requirement, Threshold},
    research::{ResearchDef, ResearcherDef},
    resource::{Banking, DepletionPolicy, HarvestData},
    rise::{RisePool, RisePoolInitial},
    skills::{
        AimParty, Away, CasterParty, Casting, Count, EntityCastEffect, FromAimTo, FromCasterTo,
        GatherSight, Gathering, Journey, PaymentPoint, PlayerCastEffect, Reach, Recipient,
        Shortfall, SkillCaster, SkillDef, Spot, SummonArrival, SummonAt, Whom,
    },
    stack_rule::StackRule,
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp},
    tags,
    terrains::Terrains,
    transport::{PassengerConduct, PassengerFate},
    tunnel::{OtherEndLost, Passage, TunnelDef},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{
    layer_id::LayerId, layer_mask::LayerMask, terrain_id::TerrainId, terrain_mask::TerrainMask,
};

//
// ─── Identity ─────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'worker' is already registered")]
fn register_rejects_duplicate_type() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN));
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN));
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
        utils::sized("gryphon", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_movement(
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
        utils::sized("wagon", utils::GROUND_DOMAIN, CellSize::new(2, 3)).with_movement(
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
    registry.register(utils::sized(
        "wall",
        utils::GROUND_DOMAIN,
        CellSize::new(3, 2),
    ));
}

//
// ─── Resource kinds ───────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_definitions_without_resources() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN));
}

#[test]
fn register_accepts_registered_resource_kinds() {
    gold_registry_with(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_price([("gold", 10)])
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_resource_carrier([(
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
            )])
            .with_resource_storage(["gold"]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'worker' harvests 'gold' it is a source of and so works it in the open"
)]
fn validate_rejects_source_harvesting_itself_from_inside() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
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
            )]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its price")]
fn register_rejects_unknown_price_kind() {
    gold_registry_with(utils::standing("worker", utils::GROUND_DOMAIN).with_price([("wood", 10)]));
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource source")]
fn register_rejects_unknown_source_kind() {
    gold_registry_with(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_resource_source("wood", DepletionPolicy::Destroy),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource carrier")]
fn register_rejects_unknown_carrier_kind() {
    gold_registry_with(
        utils::standing("worker", utils::GROUND_DOMAIN).with_resource_carrier([(
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
        )]),
    );
}

#[test]
#[should_panic(expected = "unregistered resource kind 'wood' in its resource storage")]
fn register_rejects_unknown_storage_kind() {
    gold_registry_with(
        utils::standing("worker", utils::GROUND_DOMAIN).with_resource_storage(["gold", "wood"]),
    );
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

    registry.register(utils::standing("soldier", utils::GROUND_DOMAIN).with_train_time(4));
    registry.register(
        utils::sized("depot", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_rise(6, []),
    );
    registry.register(
        utils::sized("barracks", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_trainer(["soldier"]),
    );
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
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
        utils::sized("depot", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_rise(6, [])
            .with_berths([(
                "rim",
                BerthGroup::new([point("0.5", "0.5"), point("1.5", "1.5")], 2),
            )]),
    );
    registry.register(
        utils::sized("vein", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_berths([("rim", BerthGroup::new([point("0.5", "0.5")], 1))]),
    );
    registry.register(
        utils::standing("sprite", utils::GROUND_DOMAIN)
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
    registry.register(
        utils::sized("depot", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_rise(6, []),
    );
    registry.register(
        utils::standing("sprite", utils::GROUND_DOMAIN)
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
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("sprite", utils::GROUND_DOMAIN)
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
        utils::sized("mine", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::sized("shaft_house", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_rise(6, [])
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
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::sized("shaft_house", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_rise(6, [])
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
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
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
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
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
    registry.register(utils::standing("boulder", utils::GROUND_DOMAIN));
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
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
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
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
    let aloft = registry.register_domain("aloft", DomainDef::new(air, Terrains::Any));
    registry.register(
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        EntityTypeDef::new("shaft_house")
            .with_location(aloft, CellSize::ONE, Solidity::Solid, air)
            .with_rise(6, [])
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'shaft_house' overbuilds 'mine', whose domain takes terrain its own refuses"
)]
fn validate_rejects_overbuilding_source_on_terrain_overbuilder_refuses() {
    // The mine's domain takes land and water; the shaft house's land alone.
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register_terrain("water");
    let land = registry.terrain("land").unwrap();
    let dry = registry.register_domain(
        "dry",
        DomainDef::new(utils::GROUND, Terrains::Only(land.into())),
    );
    registry.register(
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        EntityTypeDef::new("shaft_house")
            .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
            .with_rise(6, [])
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
fn validate_accepts_overbuilding_source_on_fewer_terrains() {
    // The mine's domain takes land alone; the shaft house's land and water.
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register_terrain("water");
    let land = registry.terrain("land").unwrap();
    let dry = registry.register_domain(
        "dry",
        DomainDef::new(utils::GROUND, Terrains::Only(land.into())),
    );
    registry.register(
        EntityTypeDef::new("mine")
            .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'shaft_house' overbuilds 'mine', which stands underfoot")]
fn validate_rejects_overbuilding_source_standing_underfoot() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        EntityTypeDef::new("mine")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Underfoot,
                utils::GROUND,
            )
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
            .with_rise(6, [])
            .with_resource_source("gold", DepletionPolicy::Destroy)
            .with_overbuilds("mine"),
    );
    registry.validate();
}

#[test]
fn validate_accepts_overbuilding_type_that_never_rises() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("mine", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("shaft_house", utils::GROUND_DOMAIN)
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
        utils::sized("town_hall", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_rise(6, [])
            .with_trainer(["worker"]),
    );
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
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
    registry.register(
        utils::sized("barracks", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_trainer(["ghost"]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'relay' warps in 'recruit', which declares no rise")]
fn validate_rejects_warped_unit_that_never_rises() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("recruit", utils::GROUND_DOMAIN).with_train_time(10));
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        Vec::new(),
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'relay' warps in 'recruit', which is not a registered type with a train time"
)]
fn validate_rejects_warped_unit_without_train_time() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("recruit", utils::GROUND_DOMAIN).with_rise(10, Vec::new()));
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        Vec::new(),
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
fn validate_accepts_warped_unit_that_rises() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("recruit", utils::GROUND_DOMAIN)
            .with_train_time(10)
            .with_rise(10, Vec::new()),
    );
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        Vec::new(),
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'relay' warps in 'recruit' only where 'power' covers it, but 'recruit' may not be placed on it"
)]
fn validate_rejects_warp_into_field_warped_unit_may_not_stand_on() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(
        rising_recruit().with_field_placement([FieldPlacement::Forbids { field: power }]),
    );
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        vec![powered(power)],
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'relay' warps in 'recruit' only where 'power' does not cover it, but 'recruit' may be placed only on it"
)]
fn validate_rejects_warp_outside_field_warped_unit_must_stand_on() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(rising_recruit().with_field_placement([powered(power)]));
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        vec![FieldPlacement::Forbids { field: power }],
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
fn validate_accepts_warp_into_field_warped_unit_must_stand_on() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(rising_recruit().with_field_placement([powered(power)]));
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        vec![powered(power)],
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'recruit' is placed only where 'power' covers it and only where it does not"
)]
fn register_rejects_placement_both_requiring_and_forbidding_field() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(
        rising_recruit()
            .with_field_placement([powered(power), FieldPlacement::Forbids { field: power }]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'relay' warps in only where 'power' covers it and only where it does not"
)]
fn register_rejects_warp_both_requiring_and_forbidding_field() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(rising_recruit());
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        vec![powered(power), FieldPlacement::Forbids { field: power }],
        Landing::Sighted,
    ));
}

#[test]
#[should_panic(expected = "entity type 'relay' warps in 'lookout', which is an annex")]
fn validate_rejects_warped_annex() {
    let mut registry = utils::ground_registry();
    registry.register(primary(["lookout"]));
    registry.register(
        utils::standing("lookout", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
            .with_train_time(10)
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["lookout"],
        Vec::new(),
        Landing::Sighted,
    ));
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'relay' warps in within an unregistered field")]
fn register_rejects_warp_within_foreign_field() {
    let mut registry = utils::ground_registry();
    // A handle minted by another registry, which this one never registered.
    let foreign = register_power(&mut utils::ground_registry());
    registry.register(rising_recruit());
    registry.register(utils::standing("relay", utils::GROUND_DOMAIN).with_warper(
        ["recruit"],
        vec![powered(foreign)],
        Landing::Sighted,
    ));
}

#[test]
#[should_panic(expected = "skill 'raise_lookout' summons 'lookout' rising, but it is an annex")]
fn register_rejects_rising_summon_of_annex() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("lookout", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    register_summon(&mut registry, "raise_lookout", "lookout", Vec::new());
}

#[test]
#[should_panic(expected = "skill 'raise_recruit' summons within an unregistered field")]
fn register_rejects_summon_within_foreign_field() {
    let mut registry = utils::ground_registry();
    let foreign = register_power(&mut utils::ground_registry());
    registry.register(rising_recruit());
    register_summon(
        &mut registry,
        "raise_recruit",
        "recruit",
        vec![powered(foreign)],
    );
}

#[test]
#[should_panic(
    expected = "skill 'raise_recruit' summons only where 'power' covers it and only where it does not"
)]
fn register_rejects_summon_both_requiring_and_forbidding_field() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(rising_recruit());
    register_summon(
        &mut registry,
        "raise_recruit",
        "recruit",
        vec![powered(power), FieldPlacement::Forbids { field: power }],
    );
}

#[test]
#[should_panic(
    expected = "skill 'raise_recruit' summons 'recruit' only where 'power' covers it, but 'recruit' may not be placed on it"
)]
fn register_rejects_summon_into_field_summoned_type_may_not_stand_on() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(
        rising_recruit().with_field_placement([FieldPlacement::Forbids { field: power }]),
    );
    register_summon(
        &mut registry,
        "raise_recruit",
        "recruit",
        vec![powered(power)],
    );
}

#[test]
#[should_panic(
    expected = "skill 'raise_recruit' summons 'recruit' only where 'power' does not cover it, but 'recruit' may be placed only on it"
)]
fn register_rejects_summon_outside_field_summoned_type_must_stand_on() {
    let mut registry = utils::ground_registry();
    let power = register_power(&mut registry);
    registry.register(rising_recruit().with_field_placement([powered(power)]));
    register_summon(
        &mut registry,
        "raise_recruit",
        "recruit",
        vec![FieldPlacement::Forbids { field: power }],
    );
}

#[test]
fn validate_accepts_type_that_both_trains_and_warps() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("recruit", utils::GROUND_DOMAIN)
            .with_train_time(10)
            .with_rise(10, Vec::new()),
    );
    registry.register(
        utils::standing("relay", utils::GROUND_DOMAIN)
            .with_trainer(["recruit"])
            .with_warper(["recruit"], Vec::new(), Landing::Sighted),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "skill 'raise_mound' summons 'mound' rising, but it declares no rise")]
fn register_rejects_rising_summon_of_type_that_never_rises() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("mound", utils::GROUND_DOMAIN));
    let mound = registry.type_id("mound").expect("just registered");
    registry.register_skill(
        "raise_mound",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Caster,
                    entity_type: mound,
                    count: Quantity::Constant(1),
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Rising,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "trains 'statue', which is not a registered trainable type")]
fn validate_rejects_untrainable_trained_type() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        utils::GROUND_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        utils::GROUND,
    ));
    registry.register(
        utils::sized("barracks", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_trainer(["statue"]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'worker' builds 'nexus', which is not a registered type that rises"
)]
fn validate_rejects_unknown_built_type() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
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
#[should_panic(expected = "builds 'statue', which is not a registered type that rises")]
fn validate_rejects_built_type_that_never_rises() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("statue").with_location(
        utils::GROUND_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        utils::GROUND,
    ));
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
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
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN).with_dying(3, utils::leaves("corpse")),
    );
    registry.validate();
}

#[test]
fn register_accepts_death_that_leaves_standing_units() {
    let mut registry = utils::ground_registry();

    registry.register(utils::standing("broodling", utils::GROUND_DOMAIN));
    registry.register(utils::standing("hive", utils::GROUND_DOMAIN).with_dying(
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
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN).with_dying(3, utils::leaves("ghost")),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'bones' is remains, but defines live-gameplay data that remains never use"
)]
fn register_rejects_remains_with_live_gameplay_data() {
    let mut registry = utils::ground_registry();
    registry.register(
        remains("bones", 200)
            .with_pool(Pool::health(10))
            .with_attack(utils::weapon(utils::GROUND), 1, 1, 1, 2, 1),
    );
}

#[test]
#[should_panic(expected = "entity type 'bones' is remains but carries no lifetime")]
fn register_rejects_remains_that_never_go_away() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("bones", utils::GROUND_DOMAIN).with_tags(["remains"]));
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
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN).with_morphs([morph_into("corpse")]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'larva' morphing into 'hatchling' wears remains")]
fn validate_rejects_transition_wearing_remains() {
    let mut registry = utils::ground_registry();

    registry.register(remains("corpse", 600));
    registry
        .register(utils::standing("hatchling", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));
    registry.register(
        utils::standing("larva", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_race("human"));
}

#[test]
#[should_panic(expected = "belongs to unregistered race 'orc'")]
fn register_rejects_unregistered_race() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_race("orc"));
}

//
// ─── Tags ─────────────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_registered_tag() {
    let mut registry = utils::ground_registry();
    registry.register_tag("flying");
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_tags(["flying"]));
}

#[test]
#[should_panic(expected = "references unregistered tag 'flying'")]
fn register_rejects_unregistered_tag() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_tags(["flying"]));
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
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_tags([tags::BUILDING]));
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
#[should_panic(expected = "entity type 'worker' is in a domain this registry never minted")]
fn register_rejects_domain_registry_never_minted() {
    ContentRegistry::default().register(utils::standing("worker", utils::GROUND_DOMAIN));
}

#[test]
#[should_panic(expected = "entity type 'griffon_rider' occupies unregistered layers")]
fn register_rejects_occupied_mask_mixing_in_unregistered_layer() {
    let mut registry = utils::ground_registry();
    registry.register(EntityTypeDef::new("griffon_rider").with_location(
        utils::GROUND_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        utils::GROUND | LayerId::new(2),
    ));
}

#[test]
#[should_panic(
    expected = "entity type 'gryphon' occupies layers 0b10 without layer 1, its domain's"
)]
fn register_rejects_occupied_mask_leaving_out_domain_layer() {
    let mut registry = utils::ground_registry();
    let air = registry.register_layer("air");
    registry.register(EntityTypeDef::new("gryphon").with_location(
        utils::GROUND_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        air,
    ));
}

//
// ─── Terrains ─────────────────────────────────────────────────────────────────
//

#[test]
fn register_terrain_assigns_bits_in_registration_order() {
    let mut registry = ContentRegistry::default();

    assert_eq!(registry.register_terrain("land"), TerrainId::new(1));
    assert_eq!(
        registry.register_terrain("shallow_water"),
        TerrainId::new(2)
    );
    assert_eq!(registry.register_terrain("deep_water"), TerrainId::new(4));
    assert_eq!(registry.terrain("shallow_water"), Some(TerrainId::new(2)));
    assert!(registry.has_terrain("land"));
    assert!(!registry.has_terrain("lava"));
    assert_eq!(*registry.registered_terrains(), 0b111);
}

#[test]
#[should_panic(expected = "terrain 'land' is already registered")]
fn register_terrain_rejects_duplicate_name() {
    let mut registry = ContentRegistry::default();
    registry.register_terrain("land");
    registry.register_terrain("land");
}

#[test]
#[should_panic(expected = "terrain name must not be empty")]
fn empty_terrain_name_panics() {
    ContentRegistry::default().register_terrain("");
}

#[test]
#[should_panic(expected = "all 32 terrain ids are already assigned")]
fn register_terrain_rejects_exhausted_ids() {
    let mut registry = ContentRegistry::default();
    for n in 0..=32 {
        registry.register_terrain(format!("terrain_{n}"));
    }
}

//
// ─── Domains ──────────────────────────────────────────────────────────────────
//

#[test]
fn domain_terrains_resolve_any_to_every_registered_terrain() {
    let mut registry = utils::ground_registry();
    let water = registry.register_terrain("deep_water");
    let land = registry.terrain("land").unwrap();
    let naval = registry.register_domain(
        "naval",
        DomainDef::new(utils::GROUND, Terrains::Only(water.into())),
    );

    let terrains = |domain| registry.domain_def(domain).terrains().mask(&registry);
    // The ground registry's `ground` is domain 0, so `naval` is 1.
    assert_eq!(naval, DomainId::new(1));
    assert_eq!(registry.domain("naval"), Some(naval));
    assert_eq!(terrains(naval), TerrainMask::from(water));
    assert_eq!(
        terrains(utils::GROUND_DOMAIN),
        TerrainMask::from(land) | water,
        "a domain over any terrain stands on every registered one"
    );
}

#[test]
#[should_panic(expected = "domain name must not be empty")]
fn empty_domain_name_panics() {
    utils::ground_registry().register_domain("", DomainDef::new(utils::GROUND, Terrains::Any));
}

#[test]
#[should_panic(expected = "domain 'ground' is already registered")]
fn register_domain_rejects_duplicate_name() {
    let mut registry = utils::ground_registry();
    registry.register_domain("ground", DomainDef::new(utils::GROUND, Terrains::Any));
}

#[test]
#[should_panic(expected = "domain 'aloft' is in unregistered layer 2")]
fn register_domain_rejects_unregistered_layer() {
    let mut registry = utils::ground_registry();
    registry.register_domain("aloft", DomainDef::new(LayerId::new(2), Terrains::Any));
}

#[test]
#[should_panic(expected = "a domain over no terrain holds nothing; declare it over \"any\"")]
fn domain_over_no_terrain_panics() {
    DomainDef::new(utils::GROUND, Terrains::Only(TerrainMask::EMPTY));
}

#[test]
#[should_panic(expected = "domain 'naval' names unregistered terrains 0b10")]
fn register_domain_rejects_unregistered_terrain() {
    let mut registry = utils::ground_registry();
    registry.register_domain(
        "naval",
        DomainDef::new(
            utils::GROUND,
            Terrains::Only(TerrainMask::from(TerrainId::new(2))),
        ),
    );
}

//
// ─── Stats ──────────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "has a non-positive speed stat")]
fn register_rejects_non_positive_speed() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ZERO,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );
}

#[test]
#[should_panic(expected = "has a non-positive supply_provided stat")]
fn register_rejects_non_positive_supply_provided() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "has a non-positive supply_cost stat")]
fn register_rejects_non_positive_supply_cost() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ZERO),
    );
}

#[test]
#[should_panic(expected = "declares aim_rate but carries no turret")]
fn register_rejects_aim_rate_without_turret() {
    // Only a gun with a bearing of its own reads a slew rate; on a body that turns
    // to shoot it is a rate its author believes in and nothing applies.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("gunner", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(3))
            .with_attack(utils::weapon(utils::GROUND), 10, 1, 1, 2, 1),
    );
}

#[test]
#[should_panic(expected = "declares attack_arc but has no weapon")]
fn register_rejects_attack_arc_without_weapon() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(60)),
    );
}

#[test]
#[should_panic(expected = "declares pivot_angle but cannot move")]
fn register_rejects_pivot_angle_without_movement() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::PIVOT_ANGLE, FixedU64::from_num(90)),
    );
}

#[test]
#[should_panic(expected = "has attack_range below its minimum of 1")]
fn register_rejects_zero_attack_range() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_attack(
        utils::weapon(utils::GROUND),
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
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_attack(
        utils::weapon(utils::GROUND),
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
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_attack(
        utils::weapon(utils::GROUND),
        10,
        1,
        1,
        2,
        0,
    ));
}

#[test]
#[should_panic(expected = "with an energy cost but no energy pool")]
fn register_rejects_energy_costed_skill_without_energy_pool() {
    let mut registry = utils::ground_registry();
    let jolt = registry.register_skill(
        "jolt",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: vec![Cost::Energy(FixedU64::from_num(25))],
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::from_num(5),
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("caster", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::from_num(5),
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("caster", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
                costs: vec![Cost::Resources(price::from([("wood", 5)]))],
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::from_num(5),
                    on: Recipient::Caster,
                },
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
                costs: vec![Cost::Resources(price::from([("gold", 25)]))],
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::from_num(5),
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("caster", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
                costs: vec![Cost::Health(FixedU64::from_num(5))],
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::from_num(5),
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(utils::standing("caster", utils::GROUND_DOMAIN).with_skills([rite]));
}

#[test]
#[should_panic(expected = "has attack_period below its minimum of 1")]
fn register_rejects_fractional_attack_period() {
    // Positive but below one whole tick: the engine reads the cycle as an integer,
    // so this would truncate to a phase the counter never reaches.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(0.5)),
    );
}

#[test]
#[should_panic(expected = "has a damage_point beyond its attack_period")]
fn register_rejects_damage_point_beyond_attack_period() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_attack(
        utils::weapon(utils::GROUND),
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
        utils::standing("archer", utils::GROUND_DOMAIN)
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
        utils::standing("scarecrow", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
            .with_attack_def(utils::GROUND, Delivery::Instant, None, Slain::Remains),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' declares 'health_regen', which belongs to the health pool; declare the pool"
)]
fn validate_rejects_pool_stat_without_its_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::HEALTH_REGEN, utils::fixed("0.5")),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'wall' declares the health pool with a maximum of 0")]
fn register_rejects_pool_with_zero_maximum() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("wall", utils::GROUND_DOMAIN).with_pool(Pool::health(0)));
}

#[test]
#[should_panic(
    expected = "entity type 'wall' starts the health pool's initial at a share of 1.5, above 1"
)]
fn register_rejects_initial_share_above_whole() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(health_starting(PoolInitial::Share(utils::fixed("1.5")))),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' starts the health pool's initial at 150, above its maximum of 100"
)]
fn register_rejects_initial_amount_above_maximum() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN).with_pool(health_starting(
            PoolInitial::Amount(FixedU64::from_num(150)),
        )),
    );
}

#[test]
#[should_panic(expected = "entity type 'wall' starts the health pool's initial at 0")]
fn register_rejects_health_starting_empty() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(health_starting(PoolInitial::Amount(FixedU64::ZERO))),
    );
}

#[test]
#[should_panic(expected = "entity type 'wall' starts the health pool following the work at 0")]
fn register_rejects_health_following_work_from_empty() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                vec![(
                    PoolId::HEALTH,
                    RisePool::FollowsWork(RisePoolInitial::Share(FixedU64::ZERO)),
                )],
            ),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' says how it holds a pool it does not declare while rising"
)]
fn register_rejects_rise_naming_undeclared_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                utils::rise_initial_pools(&[PoolId::HEALTH, PoolId::ENERGY]),
            ),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' does not say how it holds the energy pool while rising"
)]
fn register_rejects_rise_leaving_pool_unsaid() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(50))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH])),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' starts the health pool following the work at a share of 1.5, above 1"
)]
fn register_rejects_rise_share_above_whole() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                [(
                    PoolId::HEALTH,
                    RisePool::FollowsWork(RisePoolInitial::Share(utils::fixed("1.5"))),
                )],
            ),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' starts the health pool following the work at 150, above its maximum of 100"
)]
fn register_rejects_rise_amount_above_maximum() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                [(
                    PoolId::HEALTH,
                    RisePool::FollowsWork(RisePoolInitial::Amount(FixedU64::from_num(150))),
                )],
            ),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' has the health pool follow the work from its whole maximum; let it start at its initial"
)]
fn register_rejects_rise_from_whole_share() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                [(
                    PoolId::HEALTH,
                    RisePool::FollowsWork(RisePoolInitial::Share(FixedU64::ONE)),
                )],
            ),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'wall' has the health pool follow the work from its whole maximum; let it start at its initial"
)]
fn register_rejects_rise_from_whole_amount() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(
                4,
                [(
                    PoolId::HEALTH,
                    RisePool::FollowsWork(RisePoolInitial::Amount(FixedU64::from_num(100))),
                )],
            ),
    );
}

#[test]
#[should_panic(expected = "entity type 'wall' gains the health pool only once risen")]
fn register_rejects_health_gained_on_risen() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_rise(4, [(PoolId::HEALTH, RisePool::GainedOnRisen)]),
    );
}

#[test]
fn register_accepts_energy_gained_on_risen() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(50))
            .with_rise(
                4,
                [
                    (PoolId::HEALTH, RisePool::Initial),
                    (PoolId::ENERGY, RisePool::GainedOnRisen),
                ],
            ),
    );
    let wall = registry.entity("wall").and_then(|def| def.rise.as_ref());
    assert_eq!(
        wall.and_then(|rise| rise.pool(PoolId::ENERGY)),
        Some(RisePool::GainedOnRisen)
    );
}

#[test]
fn register_accepts_energy_starting_empty() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("wall", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(100),
                FixedU64::ZERO,
                FixedU64::ZERO,
                PoolInitial::Amount(FixedU64::ZERO),
            )),
    );
    let energy = registry
        .entity("wall")
        .and_then(|def| def.base_stats.pool(PoolId::ENERGY));
    assert_eq!(
        energy.map(|pool| pool.initial()),
        Some(PoolInitial::Amount(FixedU64::ZERO))
    );
}

#[test]
#[should_panic(expected = "declares repair_speed but cannot repair")]
fn register_rejects_repair_speed_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can build but is missing build_range")]
fn register_rejects_builder_without_reach() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN).with_builder(
            ["depot"],
            BuilderAttendance::Crew(WorkPresence::Hidden {
                crew: CrewLimit::ONE,
            }),
        ),
    );
}

#[test]
#[should_panic(expected = "declares build_range but cannot build")]
fn register_rejects_build_range_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "can carry resources but is missing harvest_range")]
fn register_rejects_carrier_without_reach() {
    gold_registry_with(
        utils::standing("worker", utils::GROUND_DOMAIN).with_resource_carrier([(
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
        )]),
    );
}

#[test]
#[should_panic(expected = "declares harvest_range but cannot carry resources")]
fn register_rejects_harvest_range_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE),
    );
}

#[test]
#[should_panic(expected = "points a weapon but is missing attack_range")]
fn register_rejects_attacker_without_weapon_stats() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(5))
            .with_attack_def(utils::GROUND, Delivery::Instant, None, Slain::Remains),
    );
}

//
// ─── Transport ────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "is missing load_range")]
fn register_rejects_transporter_without_reach() {
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
#[should_panic(expected = "is missing cargo_capacity")]
fn register_rejects_transporter_without_capacity() {
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
#[should_panic(expected = "declares unload_period, which nothing it does reads")]
fn register_rejects_transport_stat_without_capability() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("soldier", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2)),
    );
}

#[test]
#[should_panic(expected = "lets bodies in and so cannot declare cargo_size")]
fn register_rejects_transportable_transporter() {
    let mut registry = utils::ground_registry();
    registry.register_tag("infantry");
    registry.register(
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
        utils::standing("wagon", utils::GROUND_DOMAIN)
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
        utils::standing("footman", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.validate();
}

//
// ─── Gateways ─────────────────────────────────────────────────────────────────
//

#[test]
fn register_links_assigns_ids_in_registration_order() {
    let mut registry = ContentRegistry::default();
    let warren = registry.register_network("warren", warren_def(255));
    let nest = registry.register_network(
        "nest",
        NetworkDef::new(Kinds::Any, Affiliation::Allied, 8, PassengerFate::Eject),
    );
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));

    assert_eq!((warren.index(), nest.index(), tunnel.index()), (0, 1, 0));
    assert_eq!(registry.network("nest"), Some(nest));
    assert_eq!(registry.network_name(warren), Some("warren"));
    assert_eq!(registry.tunnel("tunnel"), Some(tunnel));
    assert_eq!(registry.tunnel_name(tunnel), Some("tunnel"));
    assert_eq!(registry.network_def(nest).boarding(), Affiliation::Allied);
    assert_eq!(
        registry.tunnel_def(tunnel).other_end_lost(),
        OtherEndLost::Stands
    );
}

#[test]
#[should_panic(expected = "network 'warren' is already registered")]
fn register_network_rejects_duplicate_name() {
    let mut registry = ContentRegistry::default();
    registry.register_network("warren", warren_def(255));
    registry.register_network("warren", warren_def(8));
}

#[test]
#[should_panic(expected = "tunnel 'tunnel' is already registered")]
fn register_tunnel_rejects_duplicate_name() {
    let mut registry = ContentRegistry::default();
    registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register_tunnel("tunnel", tunnel_def(held(8)));
}

#[test]
#[should_panic(expected = "network 'warren' holds nothing at all")]
fn register_network_rejects_no_capacity() {
    ContentRegistry::default().register_network("warren", warren_def(0));
}

#[test]
#[should_panic(expected = "network 'warren' admits only enemies, so no gateways ever link")]
fn register_network_rejects_enemy_boarding() {
    ContentRegistry::default().register_network(
        "warren",
        NetworkDef::new(Kinds::Any, Affiliation::Enemy, 8, PassengerFate::Destroy),
    );
}

#[test]
#[should_panic(expected = "tunnel 'tunnel' admits only enemies")]
fn register_tunnel_rejects_enemy_boarding() {
    ContentRegistry::default().register_tunnel(
        "tunnel",
        TunnelDef::new(
            Kinds::Any,
            Affiliation::Enemy,
            OtherEndLost::Stands,
            Passage::Crosses,
        ),
    );
}

#[test]
#[should_panic(expected = "tunnel 'tunnel' holds nothing at all")]
fn register_tunnel_rejects_held_passage_of_no_capacity() {
    ContentRegistry::default().register_tunnel("tunnel", tunnel_def(held(0)));
}

#[test]
fn register_accepts_gateways_reading_only_stats_they_use() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    // A held gateway both ways reads all four; a crossing one the boarding
    // pair; a crossing way out nothing at all.
    registry.register(leaving(entering(gateway(
        "burrow",
        Joins::Network(warren),
        Doors::Both,
    ))));
    registry.register(entering(gateway(
        "tunnel",
        Joins::Tunnel(tunnel),
        Doors::Both,
    )));
    registry.register(gateway("tunnel_exit", Joins::Tunnel(tunnel), Doors::Exit));
}

#[test]
#[should_panic(expected = "entity type 'burrow' joins an unregistered tunnel")]
fn register_rejects_gateway_of_link_minted_elsewhere() {
    let mut elsewhere = ContentRegistry::default();
    elsewhere.register_tunnel("other", tunnel_def(Passage::Crosses));
    let foreign = elsewhere.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    let mut registry = utils::ground_registry();
    registry.register(gateway("burrow", Joins::Tunnel(foreign), Doors::Exit));
}

#[test]
#[should_panic(expected = "entity type 'burrow' is missing unload_range")]
fn register_rejects_held_gateway_without_unloading_stats() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    registry.register(entering(gateway(
        "burrow",
        Joins::Network(warren),
        Doors::Both,
    )));
}

#[test]
#[should_panic(expected = "entity type 'chamber' is missing unload_range")]
fn register_rejects_held_tunnel_end_without_unloading_stats() {
    let mut registry = utils::ground_registry();
    let chamber = registry.register_tunnel("chamber", tunnel_def(held(8)));
    registry.register(entering(gateway(
        "chamber",
        Joins::Tunnel(chamber),
        Doors::Both,
    )));
}

#[test]
#[should_panic(
    expected = "entity type 'tunnel' declares unload_range, which nothing it does reads"
)]
fn register_rejects_unloading_stats_on_crossing_gateway() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(leaving(entering(gateway(
        "tunnel",
        Joins::Tunnel(tunnel),
        Doors::Both,
    ))));
}

#[test]
#[should_panic(expected = "entity type 'burrow' declares load_range, which nothing it does reads")]
fn register_rejects_boarding_stats_on_way_out() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    registry.register(leaving(entering(gateway(
        "burrow",
        Joins::Network(warren),
        Doors::Exit,
    ))));
}

#[test]
#[should_panic(
    expected = "entity type 'tunnel' declares cargo_capacity, which nothing it does reads"
)]
fn register_rejects_capacity_on_gateway() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(
        entering(gateway("tunnel", Joins::Tunnel(tunnel), Doors::Both))
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4)),
    );
}

#[test]
#[should_panic(expected = "network 'nest' lets in the gateway 'burrow'")]
fn validate_rejects_network_letting_in_another_networks_gateway() {
    let mut registry = utils::ground_registry();
    registry.register_tag("burrows");
    registry.register_tag("infantry");
    let warren = registry.register_network(
        "warren",
        NetworkDef::new(
            Kinds::tags(["infantry"]),
            Affiliation::Own,
            255,
            PassengerFate::Destroy,
        ),
    );
    let nest = registry.register_network(
        "nest",
        NetworkDef::new(
            Kinds::tags(["burrows"]),
            Affiliation::Own,
            8,
            PassengerFate::Destroy,
        ),
    );
    // A burrow of the infantry's warren would go into the nest.
    registry.register(
        leaving(entering(gateway(
            "burrow",
            Joins::Network(warren),
            Doors::Both,
        )))
        .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
        .with_tags(["burrows"]),
    );
    registry.register(leaving(gateway(
        "nest_exit",
        Joins::Network(nest),
        Doors::Exit,
    )));
    registry.validate();
}

#[test]
#[should_panic(expected = "network 'warren' lets in the gateway 'burrow'")]
fn validate_rejects_network_letting_in_its_own_gateway() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    registry.register(
        leaving(entering(gateway(
            "burrow",
            Joins::Network(warren),
            Doors::Both,
        )))
        .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "tunnel 'tunnel' lets in the gateway 'exit'")]
fn validate_rejects_tunnel_letting_in_its_own_gateway() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(
        gateway("exit", Joins::Tunnel(tunnel), Doors::Exit)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "network 'warren' lets in the gateway 'exit'")]
fn validate_rejects_network_letting_in_tunnel_gateway() {
    let mut registry = utils::ground_registry();
    registry.register_network("warren", warren_def(255));
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(
        gateway("exit", Joins::Tunnel(tunnel), Doors::Exit)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "network 'warren' has gateways, but none lets bodies out")]
fn validate_rejects_network_whose_gateways_let_nobody_out() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    registry.register(entering(gateway(
        "burrow",
        Joins::Network(warren),
        Doors::Entrance,
    )));
    registry.validate();
}

#[test]
#[should_panic(expected = "network 'warren' carries tag 'critters', which is not registered")]
fn validate_rejects_unresolved_network_carries_entry() {
    let mut registry = utils::ground_registry();
    registry.register_network(
        "warren",
        NetworkDef::new(
            Kinds::tags(["critters"]),
            Affiliation::Own,
            8,
            PassengerFate::Destroy,
        ),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "tunnel 'tunnel' carries tag 'critters', which is not registered")]
fn validate_rejects_unresolved_tunnel_carries_entry() {
    let mut registry = utils::ground_registry();
    registry.register_tunnel(
        "tunnel",
        TunnelDef::new(
            Kinds::tags(["critters"]),
            Affiliation::Own,
            OtherEndLost::Stands,
            Passage::Crosses,
        ),
    );
    registry.validate();
}

#[test]
fn validate_accepts_tunnel_end_raised_by_build_summon_or_training() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(
        entering(gateway("exit", Joins::Tunnel(tunnel), Doors::Both))
            .with_pool(Pool::health(100))
            .with_rise(10, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_train_time(10),
    );
    let exit = registry.type_id("exit").expect("just registered");
    let dig = registry.register_skill("dig", summoning_end(exit, 1));
    registry.register(
        entering(gateway("built", Joins::Tunnel(tunnel), Doors::Both))
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(40))
            .with_builder(["exit"], BuilderAttendance::Unattended),
    );
    registry.register(
        entering(gateway("summoned", Joins::Tunnel(tunnel), Doors::Both)).with_skills([dig]),
    );
    registry.register(
        entering(gateway("trained", Joins::Tunnel(tunnel), Doors::Both)).with_trainer(["exit"]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_tunnel_end_raised_by_warp() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(
        entering(gateway("exit", Joins::Tunnel(tunnel), Doors::Both))
            .with_pool(Pool::health(100))
            .with_rise(10, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_train_time(10),
    );
    registry.register(
        entering(gateway("warped", Joins::Tunnel(tunnel), Doors::Both)).with_warper(
            ["exit"],
            Vec::new(),
            Landing::Sighted,
        ),
    );
    registry.validate();
}

#[test]
fn validate_accepts_one_way_tunnel() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(gateway("exit", Joins::Tunnel(tunnel), Doors::Exit));
    let exit = registry.type_id("exit").expect("just registered");
    let dig = registry.register_skill("dig", summoning_end(exit, 1));
    registry.register(
        entering(gateway("entrance", Joins::Tunnel(tunnel), Doors::Entrance)).with_skills([dig]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "tunnel 'tunnel' has ends, but none builds, summons, trains or warps in another"
)]
fn validate_rejects_tunnel_whose_ends_never_raise_another() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(entering(gateway(
        "entrance",
        Joins::Tunnel(tunnel),
        Doors::Both,
    )));
    registry.register(entering(gateway(
        "exit",
        Joins::Tunnel(tunnel),
        Doors::Both,
    )));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "tunnel 'tunnel': 'entrance' raises 'exit', but neither end lets bodies in where the other lets them out"
)]
fn validate_rejects_tunnel_both_of_whose_ends_only_let_bodies_in() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(entering(gateway(
        "exit",
        Joins::Tunnel(tunnel),
        Doors::Entrance,
    )));
    let exit = registry.type_id("exit").expect("just registered");
    let dig = registry.register_skill("dig", summoning_end(exit, 1));
    registry.register(
        entering(gateway("entrance", Joins::Tunnel(tunnel), Doors::Entrance)).with_skills([dig]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_change_of_form_into_gateway() {
    let mut registry = utils::ground_registry();
    let warren = registry.register_network("warren", warren_def(255));
    registry.register(leaving(entering(gateway(
        "burrow",
        Joins::Network(warren),
        Doors::Both,
    ))));
    registry.register(
        utils::standing("pupa", utils::GROUND_DOMAIN).with_morphs([morph_into("burrow")]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "skill 'dig' summons tunnel end 'exit' in a count other than one")]
fn register_rejects_summon_of_several_tunnel_ends() {
    let mut registry = utils::ground_registry();
    let tunnel = registry.register_tunnel("tunnel", tunnel_def(Passage::Crosses));
    registry.register(entering(gateway(
        "exit",
        Joins::Tunnel(tunnel),
        Doors::Both,
    )));
    let exit = registry.type_id("exit").expect("just registered");
    registry.register_skill("dig", summoning_end(exit, 2));
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
        utils::standing("priest", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                },
            ]))],
            lasting: Lasting::For(10),
            stack_rule: StackRule::Refresh,
            interrupted_by: Vec::new(),
        },
    );
    utils::ground_registry().register_skill(
        "focus",
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::ApplyBuff {
                    buff,
                    on: Recipient::Caster,
                },
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
                price: price::from([("gold", 25)]),
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
        utils::standing("caster", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_skills([war_cry]),
    );
}

//
// ─── Repair capability ────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'medic' repairs itself and so mends in the open")]
fn validate_rejects_self_repairer_mending_from_inside() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("medic", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                Kinds::Any,
                RepairRate::Production,
                WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                },
                true,
                RepairCost::Free,
                None,
            ),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'medic' repairs itself and so mends in the open")]
fn validate_rejects_self_repairer_seated_on_itself() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("medic", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
            .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
            .with_repairer(
                Kinds::Any,
                RepairRate::Production,
                WorkPresence::Attached(Attachment::new("rim", BerthStance::Still)),
                true,
                RepairCost::Free,
                None,
            ),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "can repair but is missing repair_speed")]
fn register_rejects_repairer_without_repair_speed() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
        utils::standing("mechanic", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
    registry.register(utils::standing("tank", utils::GROUND_DOMAIN).with_pool(Pool::health(40)));

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'worker' repairs tag 'mechanical', which is not registered")]
fn validate_rejects_repairer_mending_unknown_tag() {
    // "building" is pre-registered, so an unknown tag has to be one content would
    // have had to declare itself.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
        utils::standing("worker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
#[should_panic(expected = "pays for repair with energy but has no energy pool")]
fn register_rejects_energy_paid_repair_without_pool() {
    let mut registry = utils::ground_registry();
    registry.register_tag("biological");
    registry.register(
        utils::standing("medic", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
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
#[should_panic(expected = "has a repair_ratio but no build or train_time")]
fn register_rejects_repair_ratio_without_production_time() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("monolith", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
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
        ResearchDef::new(price::from([("gold", 30)]), 10, Some(buff), Vec::new()),
    );
    let tactics = registry.register_research(
        "tactics",
        ResearchDef::new(Price::new(), 5, None, [Requirement::Research(smithing)]),
    );

    assert!(registry.has_research("smithing"));
    assert_eq!(registry.research("smithing"), Some(smithing));
    assert_eq!(registry.research_name(tactics), Some("tactics"));
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
    assert_eq!(registry.research_def(tactics).unwrap().buff, None);

    // Re-registering a name keeps the first definition and returns its id.
    let again = registry.register_research(
        "smithing",
        ResearchDef::new(Price::new(), 99, None, Vec::new()),
    );
    assert_eq!(again, smithing);
    assert_eq!(registry.research_def(smithing).unwrap().research_time, 10);
}

#[test]
#[should_panic(expected = "research name must not be empty")]
fn register_research_rejects_empty_name() {
    utils::ground_registry()
        .register_research("", ResearchDef::new(Price::new(), 10, None, Vec::new()));
}

#[test]
#[should_panic(expected = "research 'smithing' costs unregistered resource kind 'gold'")]
fn register_research_rejects_unknown_cost_kind() {
    utils::ground_registry().register_research(
        "smithing",
        ResearchDef::new(price::from([("gold", 30)]), 10, None, Vec::new()),
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
        ResearchDef::new(Price::new(), 10, Some(buff), Vec::new()),
    );
}

#[test]
#[should_panic(expected = "research_time must be greater than 0")]
fn research_def_rejects_zero_time() {
    ResearchDef::new(Price::new(), 0, None, Vec::new());
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
        ResearchDef::new(Price::new(), 10, None, Vec::new()),
    );
    utils::ground_registry()
        .register(utils::standing("lab", utils::GROUND_DOMAIN).with_researcher([research]));
}

#[test]
#[should_panic(
    expected = "entity type 'keep' offers docks but raises its sites as a hidden builder"
)]
fn validate_rejects_primary_that_goes_inside_its_annex_site() {
    let mut registry = utils::ground_registry();
    registry.register(
        EntityTypeDef::new("keep")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::new(2, 2),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(100))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
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
#[should_panic(expected = "annex 'mast' fades but has no health pool")]
fn validate_rejects_fading_annex_without_health_pool() {
    let mut registry = utils::ground_registry();
    registry.register_tag("building");
    registry.register(
        EntityTypeDef::new("keep")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::new(2, 2),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(100))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_rise(4, [])
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
        utils::standing("seam", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", utils::GROUND_DOMAIN)
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
        utils::standing("seam", utils::GROUND_DOMAIN)
            .with_resource_source("gold", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", utils::GROUND_DOMAIN)
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
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
        utils::standing("miner", utils::GROUND_DOMAIN)
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
        utils::standing("grove", utils::GROUND_DOMAIN)
            .with_resource_source("wood", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("miner", utils::GROUND_DOMAIN)
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
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
    registry.register(utils::standing("runner", utils::GROUND_DOMAIN).with_pool(Pool::health(10)));

    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' offers a dock at (2, 0) that names no annex")]
fn validate_rejects_dock_that_names_no_annex() {
    let mut registry = utils::ground_registry();
    registry.register_tag("workshop");
    registry.register(
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
        utils::standing("runner", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH])),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'keep' offers a dock at (0, 0), inside its own footprint")]
fn validate_rejects_dock_inside_its_primarys_own_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(100))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Passable,
                utils::GROUND,
            )
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
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
        utils::standing("lookout", utils::GROUND_DOMAIN)
            .with_rise(4, [])
            .with_annex(standing_annex(), AnnexClaim::Bound),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "annex 'lookout' never rises")]
fn validate_rejects_annex_that_never_rises() {
    let mut registry = utils::ground_registry();
    // No primary: one that raises it would have to declare that it rises,
    // and registered ahead of the annex it would be `validate_builds` that
    // spoke first.
    registry.register(
        utils::standing("lookout", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::new(2, 2),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(100))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
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
            Price::new(),
            10,
            None,
            [Requirement::EntityType("lab".to_string())],
        ),
    );
    // The knight's requirements name a type registered after it, the reserved
    // "building" tag, and a research.
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([
            Requirement::EntityType("lab".to_string()),
            Requirement::Tag(tags::BUILDING.to_string()),
            Requirement::Research(smithing),
        ]),
    );
    registry.register(
        utils::standing("lab", utils::GROUND_DOMAIN)
            .with_researcher([smithing])
            .with_tags([tags::BUILDING]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires the entity type 'chapel', which is not registered"
)]
fn validate_rejects_unknown_requirement() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN)
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
        utils::GROUND_DOMAIN,
        CellSize::ONE,
        Solidity::Solid,
        utils::GROUND,
    ));
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN)
            .with_requires([Requirement::Tag("forge".to_string())]),
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
            Price::new(),
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
        ResearchDef::new(Price::new(), 10, None, Vec::new()),
    );
    let mut skill = player_cast(haste);
    skill.requires = vec![Requirement::Research(war_drums)];
    registry.register_skill("war_cry", skill);
    registry.validate();
}

#[test]
fn validate_accepts_nested_nodes_and_state_leaves() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("lab", utils::GROUND_DOMAIN));
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Any(vec![
            Requirement::EntityType("lab".to_string()),
            Requirement::All(vec![
                Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5")))),
                Requirement::Energy(Bound::Amount(Threshold::AtLeast(FixedU64::ONE))),
                Requirement::Stat {
                    stat: EntityStatId::SPEED,
                    bound: Bound::Amount(Threshold::Under(FixedU64::from_num(2))),
                },
                Requirement::Idle,
                Requirement::IdleFor(40),
                Requirement::UnhurtFor(200),
            ]),
        ])]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires a list of nothing; a requirement names at least one thing"
)]
fn validate_rejects_empty_node() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Any(vec![])]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires health at a share of 1.5 of its maximum; a pool holds at most 1"
)]
fn validate_rejects_pool_share_over_whole() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Health(
            Bound::Share(Threshold::Under(utils::fixed("1.5"))),
        )]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires energy at a share of none; a share is above 0"
)]
fn validate_rejects_share_of_none() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Energy(
            Bound::Share(Threshold::AtLeast(FixedU64::ZERO)),
        )]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'knight' requires health under nothing, which is never met")]
fn validate_rejects_amount_under_nothing() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Health(
            Bound::Amount(Threshold::Under(FixedU64::ZERO)),
        )]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires energy at least nothing, which always holds"
)]
fn validate_rejects_amount_at_least_nothing() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Energy(
            Bound::Amount(Threshold::AtLeast(FixedU64::ZERO)),
        )]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_stat_share_over_whole() {
    let mut registry = utils::ground_registry();
    // Hasted to at least half again its base speed: a stat's share is open.
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Stat {
            stat: EntityStatId::SPEED,
            bound: Bound::Share(Threshold::AtLeast(utils::fixed("1.5"))),
        }]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires idling for no ticks at all; idle_for is at least one"
)]
fn validate_rejects_idle_for_zero() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::IdleFor(0)]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'knight' requires no hit for no ticks at all; unhurt_for is at least one"
)]
fn validate_rejects_unhurt_for_zero() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::UnhurtFor(0)]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'swollen' holds while it judges 'max_health', which its own modifiers move"
)]
fn validate_rejects_while_buff_moving_stat_it_is_judged_on() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                }],
                pool_shift: PoolShift::Share,
            })],
            lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::AtLeast(
                utils::fixed("0.5"),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'charged' holds while it judges 'max_energy', which its own modifiers move"
)]
fn validate_rejects_while_buff_on_energy_share_moving_maximum() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "charged",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_ENERGY,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(10),
                }],
                pool_shift: PoolShift::Share,
            })],
            lasting: Lasting::While(Requirement::Energy(Bound::Share(Threshold::AtLeast(
                utils::fixed("0.5"),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'sprint' holds while it judges 'speed', which its own modifiers move"
)]
fn validate_rejects_while_buff_on_stat_amount_moving_that_stat() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "sprint",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::ONE,
                },
            ]))],
            lasting: Lasting::While(Requirement::Stat {
                stat: EntityStatId::SPEED,
                bound: Bound::Amount(Threshold::Under(FixedU64::ONE)),
            }),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_lowering_maximum_under_its_line() {
    let mut registry = utils::ground_registry();
    // Under 100 health lowers a 200 maximum by 60% to 80 by share: the
    // amount falls with it and stays under the line, so it lapses only once
    // healed past 100 of its own.
    registry.register_entity_buff(
        "crippled",
        held_on_health_buff(
            "-0.6",
            PoolShift::Share,
            Threshold::Under(FixedU64::from_num(100)),
        ),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'crippled' holds on an amount of a pool capped by 'max_health', which its own modifiers shift past its line"
)]
fn validate_rejects_while_buff_lowering_pool_past_at_least_line() {
    let mut registry = utils::ground_registry();
    // At least 100 health lowers the maximum by share: the amount falls
    // under the line, the buff lapses, and the amount comes back.
    registry.register_entity_buff(
        "crippled",
        held_on_health_buff(
            "-0.6",
            PoolShift::Share,
            Threshold::AtLeast(FixedU64::from_num(100)),
        ),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'channeler' holds passive 'surging' → passive 'steeled' → passive 'surging' on requirements that feed each other"
)]
fn validate_rejects_passives_feeding_each_other() {
    let mut registry = utils::ground_registry();
    // Under 50 health raises the energy maximum by share; under 60 energy
    // raises the health maximum by difference: each moves the amount the
    // other judges, so the pair flips every tick.
    let surging = registry.register_entity_buff(
        "surging",
        held_on_amount_buff(
            Requirement::Health(Bound::Amount(Threshold::Under(FixedU64::from_num(50)))),
            EntityStatId::MAX_ENERGY,
            "0.5",
            PoolShift::Share,
        ),
    );
    let steeled = registry.register_entity_buff(
        "steeled",
        held_on_amount_buff(
            Requirement::Energy(Bound::Amount(Threshold::Under(FixedU64::from_num(60)))),
            EntityStatId::MAX_HEALTH,
            "100",
            PoolShift::Difference,
        ),
    );
    registry.register(
        utils::standing("channeler", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(100))
            .with_passives([surging, steeled]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'channeler' holds passive 'surging' → field effect 1 → passive 'surging' on requirements that feed each other"
)]
fn validate_rejects_passive_and_field_effect_feeding_each_other() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    // The passive raises the energy maximum by share while under 50 health;
    // the field effect raises the health maximum by difference while under
    // 60 energy.
    let surging = registry.register_entity_buff(
        "surging",
        held_on_amount_buff(
            Requirement::Health(Bound::Amount(Threshold::Under(FixedU64::from_num(50)))),
            EntityStatId::MAX_ENERGY,
            "0.5",
            PoolShift::Share,
        ),
    );
    registry.register(
        utils::standing("channeler", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(100))
            .with_passives([surging])
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Any,
                vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![EntityModifier {
                        stat: EntityStatId::MAX_HEALTH,
                        op: ModifierOp::FlatAdd,
                        magnitude: FixedI64::from_num(100),
                    }],
                    pool_shift: PoolShift::Difference,
                })],
                Vec::new(),
                Some(Requirement::Energy(Bound::Amount(Threshold::Under(
                    FixedU64::from_num(60),
                )))),
            )]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'runner' holds passive 'hasted' → passive 'steadied' → passive 'hasted' on requirements that feed each other"
)]
fn validate_rejects_passives_feeding_each_other_through_stats_they_read() {
    let mut registry = utils::ground_registry();
    // Each reads the stat the other moves: armor and sight.
    let hasted = registry.register_entity_buff(
        "hasted",
        held_on_stat_buff(EntityStatId::SIGHT_RANGE, EntityStatId::ARMOR),
    );
    let steadied = registry.register_entity_buff(
        "steadied",
        held_on_stat_buff(EntityStatId::ARMOR, EntityStatId::SIGHT_RANGE),
    );
    registry.register(
        utils::standing("runner", utils::GROUND_DOMAIN)
            .with_stat(EntityStatId::ARMOR, FixedU64::ONE)
            .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(5))
            .with_passives([hasted, steadied]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'crippled' holds on an amount of a pool capped by 'max_health', which its own modifiers shift past its line"
)]
fn validate_rejects_while_buff_lowering_by_difference_under_at_least_line() {
    let mut registry = utils::ground_registry();
    // At least 100 health lowers the maximum by 60% by difference: the amount
    // falls by the same and drops under the line.
    registry.register_entity_buff(
        "crippled",
        held_on_health_buff(
            "-0.6",
            PoolShift::Difference,
            Threshold::AtLeast(FixedU64::from_num(100)),
        ),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'crippled' holds on an amount of a pool capped by 'max_health', which its own modifiers shift past its line"
)]
fn validate_rejects_while_buff_lowering_pool_past_under_line_turned_by_unless() {
    let mut registry = utils::ground_registry();
    // Held unless under 100 health — met at 100 and over — the share lowering
    // takes the amount under the line.
    let mut buff = held_on_health_buff(
        "-0.6",
        PoolShift::Share,
        Threshold::Under(FixedU64::from_num(100)),
    );
    buff.lasting = Lasting::While(Requirement::Unless(Box::new(Requirement::Health(
        Bound::Amount(Threshold::Under(FixedU64::from_num(100))),
    ))));
    registry.register_entity_buff("crippled", buff);
    registry.validate();
}

#[test]
fn validate_accepts_passives_looping_through_clamp_lowerings() {
    let mut registry = utils::ground_registry();
    // Each lowers the maximum of the pool the other judges, by clamp: a cut
    // that lapses gives nothing back, so the pair settles.
    let sapping = registry.register_entity_buff(
        "sapping",
        held_on_amount_buff(
            Requirement::Health(Bound::Amount(Threshold::Under(FixedU64::from_num(50)))),
            EntityStatId::MAX_ENERGY,
            "-20",
            PoolShift::Clamp,
        ),
    );
    let draining = registry.register_entity_buff(
        "draining",
        held_on_amount_buff(
            Requirement::Energy(Bound::Amount(Threshold::Under(FixedU64::from_num(60)))),
            EntityStatId::MAX_HEALTH,
            "-20",
            PoolShift::Clamp,
        ),
    );
    registry.register(
        utils::standing("channeler", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(100))
            .with_passives([sapping, draining]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_passive_feeding_one_that_feeds_nothing() {
    let mut registry = utils::ground_registry();
    // The first moves the energy the second judges; the second moves the
    // health maximum by clamp upward, which leaves every amount where it is.
    let surging = registry.register_entity_buff(
        "surging",
        held_on_amount_buff(
            Requirement::Health(Bound::Amount(Threshold::Under(FixedU64::from_num(50)))),
            EntityStatId::MAX_ENERGY,
            "0.5",
            PoolShift::Share,
        ),
    );
    let steeled = registry.register_entity_buff(
        "steeled",
        held_on_amount_buff(
            Requirement::Energy(Bound::Amount(Threshold::Under(FixedU64::from_num(60)))),
            EntityStatId::MAX_HEALTH,
            "100",
            PoolShift::Clamp,
        ),
    );
    registry.register(
        utils::standing("channeler", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::energy(100))
            .with_passives([surging, steeled]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_lowering_by_clamp_under_at_least_line() {
    let mut registry = utils::ground_registry();
    // A clamp lowering may cut the amount under the line once; the lapse
    // gives nothing back, so the buff stays off.
    registry.register_entity_buff(
        "crippled",
        held_on_health_buff(
            "-0.6",
            PoolShift::Clamp,
            Threshold::AtLeast(FixedU64::from_num(100)),
        ),
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_on_amount_raising_maximum() {
    let mut registry = utils::ground_registry();
    // An amount of health is capped by the maximum, so a buff held at least
    // on one may raise it: a higher amount keeps the requirement met.
    registry.register_entity_buff(
        "rallied",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                }],
                pool_shift: PoolShift::Share,
            })],
            lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::AtLeast(
                FixedU64::from_num(50),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_moving_pool_under_its_line() {
    let mut registry = utils::ground_registry();
    let on_fire = registry.register_entity_buff(
        "on_fire",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::HEALTH_DRAIN,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(2),
                },
            ]))],
            lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
                utils::fixed("0.34"),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_passives([on_fire]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'depot' bears passive 'on_fire', which modifies 'armor' it does not carry"
)]
fn validate_rejects_passive_modifying_stat_bearer_lacks() {
    let mut registry = utils::ground_registry();
    let on_fire = registry.register_entity_buff(
        "on_fire",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::ARMOR,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(2),
                },
            ]))],
            lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
                utils::fixed("0.34"),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_passives([on_fire]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity buff 'swell' moves 'max_health' but declares no pool shift")]
fn register_rejects_buff_on_pool_maximum_without_shift() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swell",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(10),
                },
            ]))],
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
}

#[test]
#[should_panic(
    expected = "entity buff 'haste' declares a pool shift but moves 'speed', no pool maximum"
)]
fn register_rejects_shift_on_buff_moving_no_pool_maximum() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "haste",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(1),
                }],
                pool_shift: PoolShift::Share,
            })],
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "player buff 'empty' lays a set of no modifiers")]
fn register_rejects_player_buff_laying_empty_set() {
    let mut registry = utils::ground_registry();
    registry.register_player_buff(
        "empty",
        PlayerBuffDef {
            player_modifiers: Vec::new(),
            entity_modifiers: vec![EntityModifiers::Stats(Vec::new())],
            duration: None,
            stack_rule: StackRule::Ignore,
        },
    );
}

#[test]
#[should_panic(expected = "player buff 'drill' moves 'max_energy' but declares no pool shift")]
fn register_rejects_player_buff_on_pool_maximum_without_shift() {
    let mut registry = utils::ground_registry();
    registry.register_player_buff(
        "drill",
        PlayerBuffDef {
            player_modifiers: Vec::new(),
            entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                stat: EntityStatId::MAX_ENERGY,
                op: ModifierOp::FlatAdd,
                magnitude: FixedI64::from_num(10),
            }])],
            duration: None,
            stack_rule: StackRule::Ignore,
        },
    );
}

#[test]
#[should_panic(
    expected = "entity type 'shrub' field effect moves 'max_health' but declares no pool shift"
)]
fn register_rejects_field_effect_on_pool_maximum_without_shift() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("shrub", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(35))
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Every,
                Vec::new(),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    EntityModifier {
                        stat: EntityStatId::MAX_HEALTH,
                        op: ModifierOp::PercentAdd,
                        magnitude: -FixedI64::ONE,
                    },
                ]))],
                None,
            )]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'shrub' field effect holds while it judges 'max_health', which its own modifiers move"
)]
fn register_rejects_field_effect_moving_stat_it_holds_while_judging() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("shrub", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(35))
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Every,
                Vec::new(),
                vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![EntityModifier {
                        stat: EntityStatId::MAX_HEALTH,
                        op: ModifierOp::PercentAdd,
                        magnitude: -FixedI64::ONE,
                    }],
                    pool_shift: PoolShift::Share,
                })],
                Some(Requirement::Health(Bound::Share(Threshold::AtLeast(
                    utils::fixed("0.5"),
                )))),
            )]),
    );
}

#[test]
fn validate_accepts_passive_on_rate_its_pool_carries() {
    let mut registry = utils::ground_registry();
    // The depot names no drain, but its health pool carries one.
    let on_fire = registry.register_entity_buff(
        "on_fire",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::HEALTH_DRAIN,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(2),
                },
            ]))],
            lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
                utils::fixed("0.34"),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_passives([on_fire]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'lit' requires the entity type 'beacon', which is not registered"
)]
fn validate_rejects_while_buff_naming_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "lit",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::EntityType("beacon".to_string())),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'lit' requires the entity type 'beacon', which is not registered"
)]
fn validate_rejects_unresolved_name_under_unless() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "lit",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::Unless(Box::new(Requirement::EntityType(
                "beacon".to_string(),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'swollen' holds while it judges 'max_health', which its own modifiers move"
)]
fn validate_rejects_while_buff_moving_stat_it_judges_under_unless() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                }],
                pool_shift: PoolShift::Share,
            })],
            lasting: Lasting::While(Requirement::Unless(Box::new(Requirement::Health(
                Bound::Share(Threshold::AtLeast(utils::fixed("0.5"))),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'swollen' holds on an amount of a pool capped by 'max_health', which its own modifiers shift past its line"
)]
fn validate_rejects_while_buff_shifting_pool_whose_amount_it_judges() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(50),
                }],
                pool_shift: PoolShift::Difference,
            })],
            lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::Under(
                FixedU64::from_num(100),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'swollen' holds on an amount of a pool capped by 'max_health', which its own modifiers shift past its line"
)]
fn validate_rejects_while_buff_shifting_pool_past_line_turned_by_unless() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(50),
                }],
                pool_shift: PoolShift::Difference,
            })],
            lasting: Lasting::While(Requirement::Unless(Box::new(Requirement::Health(
                Bound::Amount(Threshold::AtLeast(FixedU64::from_num(100))),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_shifting_pool_away_from_its_line() {
    let mut registry = utils::ground_registry();
    // Held while alive: raising the pool keeps it alive.
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(50),
                }],
                pool_shift: PoolShift::Difference,
            })],
            lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::AtLeast(
                FixedU64::ONE,
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
fn validate_accepts_while_buff_raising_pool_by_clamp_whose_amount_it_judges() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "swollen",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(50),
                }],
                pool_shift: PoolShift::Clamp,
            })],
            lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::Under(
                FixedU64::from_num(100),
            )))),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'depot' bears passive 'thick_hide', which does not hold on a requirement"
)]
fn validate_rejects_passive_not_holding_on_requirement() {
    let mut registry = utils::ground_registry();
    let thick_hide = registry.register_entity_buff(
        "thick_hide",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                EntityModifier {
                    stat: EntityStatId::ARMOR,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::ONE,
                },
            ]))],
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
            .with_passives([thick_hide]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'depot' bears a passive this registry never registered")]
fn validate_rejects_passive_registered_elsewhere() {
    // A buff minted by another registry: its index names nothing here.
    let foreign = utils::ground_registry().register_entity_buff(
        "lit",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::Idle),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_passives([foreign]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'depot' bears passive 'on_fire' twice")]
fn validate_rejects_passive_named_twice() {
    let mut registry = utils::ground_registry();
    let on_fire = registry.register_entity_buff(
        "on_fire",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::Idle),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.register(
        utils::standing("depot", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_passives([on_fire, on_fire]),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "entity type 'knight' requires a stat this registry never registered")]
fn validate_rejects_stat_requirement_on_stat_registered_elsewhere() {
    // A stat minted by another registry: its index is one past this one's
    // built-ins, which this registry never registered.
    let girth = utils::ground_registry().register_entity_stat("girth", FixedU64::ZERO);
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("knight", utils::GROUND_DOMAIN).with_requires([Requirement::Stat {
            stat: girth,
            bound: Bound::Amount(Threshold::Under(FixedU64::ONE)),
        }]),
    );
    registry.validate();
}

//
// ─── Unlockability ────────────────────────────────────────────────────────────
//

// A requirement is met in some order of unlocking, from the things that
// require nothing up. A cycle with no way in is refused; an `any` with a free
// branch is a way in.

#[test]
#[should_panic(
    expected = "entity type 'chapel' requires the entity type 'keep', which can never be unlocked"
)]
fn validate_rejects_two_types_requiring_each_other() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("chapel", utils::GROUND_DOMAIN)
            .with_requires([Requirement::EntityType("keep".to_string())]),
    );
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN)
            .with_requires([Requirement::EntityType("chapel".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'keep' requires the research 'masonry', which can never be unlocked"
)]
fn validate_rejects_research_and_type_requiring_each_other() {
    let mut registry = utils::ground_registry();
    let masonry = registry.register_research(
        "masonry",
        ResearchDef::new(
            Price::new(),
            5,
            None,
            [Requirement::EntityType("keep".to_string())],
        ),
    );
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN)
            .with_requires([Requirement::Research(masonry)])
            .with_researcher([masonry]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "research 'masonry' requires a type carrying the tag 'hall', which can never be unlocked"
)]
fn validate_rejects_research_requiring_tag_nothing_carries() {
    let mut registry = utils::ground_registry();
    registry.register_tag("hall");
    let masonry = registry.register_research(
        "masonry",
        ResearchDef::new(
            Price::new(),
            5,
            None,
            [Requirement::Tag("hall".to_string())],
        ),
    );
    registry.register(utils::standing("lab", utils::GROUND_DOMAIN).with_researcher([masonry]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "skill 'haste' requires a type carrying the tag 'hall', which can never be unlocked"
)]
fn validate_rejects_skill_requiring_tag_nothing_carries() {
    let mut registry = utils::ground_registry();
    registry.register_tag("hall");
    let buff = haste_buff(&mut registry);
    let mut skill = player_cast(buff);
    skill.requires = vec![Requirement::Tag("hall".to_string())];
    registry.register_skill("haste", skill);
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hall' changing into 'keep' requires a type carrying the tag 'forge', which can never be unlocked"
)]
fn validate_rejects_change_of_form_requiring_tag_nothing_carries() {
    let mut registry = utils::ground_registry();
    registry.register_tag("forge");
    registry.register(utils::standing("keep", utils::GROUND_DOMAIN));
    registry.register(utils::standing("hall", utils::GROUND_DOMAIN).with_morphs([
        MorphTransition::new(
            "keep",
            MorphCourse::direct(MorphInterrupted::Reverts),
            Quantity::Constant(20),
            MorphPlacement::Reserve,
            MorphCancel::Committed,
            MorphReason::Change,
            Vec::new(),
            [Requirement::Tag("forge".to_string())],
            [],
        ),
    ]));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'chapel' requires a type carrying the tag 'holy', which can never be unlocked"
)]
fn validate_rejects_cycle_through_tag() {
    let mut registry = utils::ground_registry();
    registry.register_tag("holy");
    // The chapel needs something holy; the only holy thing needs the chapel.
    registry.register(
        utils::standing("chapel", utils::GROUND_DOMAIN)
            .with_requires([Requirement::Tag("holy".to_string())]),
    );
    registry.register(
        utils::standing("shrine", utils::GROUND_DOMAIN)
            .with_tags(["holy"])
            .with_requires([Requirement::EntityType("chapel".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'chapel' requires one of the entity type 'keep', a type carrying the tag 'holy', which can never be unlocked"
)]
fn validate_names_every_dead_branch_of_any() {
    let mut registry = utils::ground_registry();
    registry.register_tag("holy");
    registry.register(
        utils::standing("chapel", utils::GROUND_DOMAIN).with_requires([Requirement::Any(vec![
            Requirement::EntityType("keep".to_string()),
            Requirement::Tag("holy".to_string()),
        ])]),
    );
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN)
            .with_requires([Requirement::EntityType("chapel".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'keep' requires 'lookout' docked, which can never be unlocked"
)]
fn validate_rejects_annex_requirement_that_can_never_be_unlocked() {
    let mut registry = utils::ground_registry();
    registry.register(
        EntityTypeDef::new("keep")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::new(2, 2),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(100))
            .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
            .with_builder(
                ["lookout"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            )
            .with_docks([(CellPos::new(2, 0), Kinds::types(["lookout"]))])
            .with_requires([Requirement::Annexed("lookout".to_string())]),
    );
    registry.register(
        EntityTypeDef::new("lookout")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(10))
            .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
            .with_annex(
                AloneConduct::Standing {
                    work: AnnexWork::Works,
                    life: AnnexLife::Endures,
                },
                AnnexClaim::Bound,
            )
            .with_requires([Requirement::EntityType("keep".to_string())]),
    );
    registry.validate();
}

#[test]
fn validate_accepts_cycle_with_free_branch() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("hall", utils::GROUND_DOMAIN));
    // chapel needs keep or hall; keep needs chapel. The hall lets chapel in,
    // and chapel lets keep in.
    registry.register(
        utils::standing("chapel", utils::GROUND_DOMAIN).with_requires([Requirement::Any(vec![
            Requirement::EntityType("keep".to_string()),
            Requirement::EntityType("hall".to_string()),
        ])]),
    );
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN)
            .with_requires([Requirement::EntityType("chapel".to_string())]),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity buff 'lit' requires a type carrying the tag 'hall', which can never be unlocked"
)]
fn validate_rejects_while_buff_on_tag_nothing_carries() {
    let mut registry = utils::ground_registry();
    registry.register_tag("hall");
    registry.register_entity_buff(
        "lit",
        EntityBuffDef {
            effects: vec![EntityEffect::Conceal],
            lasting: Lasting::While(Requirement::Tag("hall".to_string())),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
    registry.validate();
}

#[test]
fn validate_accepts_bonus_against_type_or_tag() {
    // Either kind resolves, and a bonus may name a type registered after the type
    // that fears it — which is why this is judged after registration, not during.
    let mut registry = utils::ground_registry();
    registry.register_tag("armored");
    registry.register(
        utils::standing("archer", utils::GROUND_DOMAIN)
            .with_bonus_damage_vs([("armored", 4u32), ("keep", 6u32)])
            .with_attack(utils::weapon(utils::GROUND), 6, 4, 4, 7, 3),
    );
    registry.register(utils::standing("keep", utils::GROUND_DOMAIN));

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
        utils::standing("archer", utils::GROUND_DOMAIN)
            .with_bonus_damage_vs([("sieging", 4u32)])
            .with_attack(utils::weapon(utils::GROUND), 6, 4, 4, 7, 3),
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
        utils::sized("walker", utils::GROUND_DOMAIN, CellSize::new(2, 2))
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
        utils::sized("flier", utils::GROUND_DOMAIN, CellSize::new(2, 2))
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
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([morph_into("flier")]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' carries the energy pool on its landing, which 'flier' does not have"
)]
fn validate_rejects_landing_carry_of_pool_destination_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20))
            .with_morphs([carrying_into(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                [(PoolId::ENERGY, PoolCarry::Full)],
            )]),
    );
    registry.register(utils::standing("flier", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' carries the energy pool on its entering, which 'cocoon' does not have"
)]
fn validate_rejects_entering_carry_of_pool_interim_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("cocoon", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20))
            .with_morphs([carrying_into(
                "flier",
                MorphCourse::via(
                    "cocoon",
                    [(PoolId::ENERGY, PoolCarry::Full)],
                    ViaInterrupted::Dies,
                ),
                [],
            )]),
    );
    registry.register(utils::standing("flier", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' carries the energy pool on its revert, which 'walker' does not have"
)]
fn validate_rejects_revert_carry_of_pool_origin_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("cocoon", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20)),
    );
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_morphs([carrying_into(
                "flier",
                MorphCourse::via(
                    "cocoon",
                    [],
                    ViaInterrupted::reverts([(PoolId::ENERGY, RevertCarry::Restore)]),
                ),
                [],
            )]),
    );
    registry.register(utils::standing("flier", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' carries the energy pool on its landing, which none of 'cocoon', 'walker' had"
)]
fn validate_rejects_landing_carry_of_pool_no_form_left_had() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("cocoon", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_morphs([carrying_into(
                "flier",
                MorphCourse::via("cocoon", [], ViaInterrupted::Dies),
                [(PoolId::ENERGY, PoolCarry::Full)],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20)),
    );
    registry.validate();
}

#[test]
fn validate_accepts_landing_carry_of_pool_interim_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("cocoon", utils::GROUND_DOMAIN).with_pool(Pool::health(50)));
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20))
            .with_morphs([carrying_into(
                "flier",
                MorphCourse::via("cocoon", [], ViaInterrupted::Dies),
                [(PoolId::ENERGY, PoolCarry::Shift(PoolShift::Share))],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(20)),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' names a type that is not registered"
)]
fn validate_rejects_transition_into_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
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
#[should_panic(
    expected = "entity type 'walker' morphing into 'keep' lands nearby, which only a form that can move does"
)]
fn validate_rejects_nearby_landing_into_form_that_cannot_move() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("keep", utils::GROUND_DOMAIN));
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "keep",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(20),
                MorphPlacement::Nearby,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [],
            )]),
    );

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_odd_footprint_difference() {
    // 1x1 -> 2x2 recentres by half a cell; the landing settles the anchor
    // onto the lattice where the model needs it, so nothing is refused here.
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
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
        utils::sized("giant", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_movement(
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
        utils::standing("walker", utils::GROUND_DOMAIN)
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
        utils::sized("giant", utils::GROUND_DOMAIN, CellSize::new(3, 3)).with_movement(
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
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                [Requirement::EntityType("jet_pack".to_string())],
                [],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

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
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Stat(stat),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'walker' morphing into 'flier' has an energy cost but no \
                energy pool"
)]
fn validate_rejects_transition_with_energy_cost_but_no_energy_pool() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphReason::Change,
                vec![Cost::Energy(FixedU64::from_num(20))],
                Vec::new(),
                [],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

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
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_morphs([MorphTransition::new(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(20),
                MorphPlacement::Revalidate,
                MorphCancel::Committed,
                MorphReason::Change,
                vec![Cost::Resources(price::from([("gold", 50)]))],
                Vec::new(),
                [],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

#[test]
#[should_panic(expected = "wears a form that is not registered")]
fn validate_rejects_transition_through_unregistered_form() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry
        .register(utils::standing("hatchling", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));

    registry.validate();
}

#[test]
#[should_panic(expected = "wears a form whose footprint differs from its own")]
fn validate_rejects_transition_through_form_of_other_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(
        utils::sized("egg", utils::GROUND_DOMAIN, CellSize::new(2, 2)).with_pool(Pool::health(60)),
    );
    registry
        .register(utils::standing("hatchling", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));

    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'larva' morphing into 'hatchling' wears a form whose domain refuses terrain its own takes"
)]
fn validate_rejects_transition_through_form_on_fewer_terrains() {
    // The larva's domain takes every terrain, the egg's dry land alone: worn
    // where the larva stands, the egg could stand on water it may not be on.
    let mut registry = utils::ground_registry();
    registry.register_terrain("water");
    let dry = registry.register_domain(
        "dry",
        DomainDef::new(
            utils::GROUND,
            Terrains::Only(registry.terrain("land").unwrap().into()),
        ),
    );
    registry.register(
        utils::standing("larva", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(
        EntityTypeDef::new("egg")
            .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
            .with_pool(Pool::health(60)),
    );
    registry
        .register(utils::standing("hatchling", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));

    registry.validate();
}

#[test]
fn validate_accepts_transition_through_form_on_more_terrains() {
    // The larva's and the hatchling's domain takes dry land alone, the egg's
    // every terrain: worn where the larva stands, the egg stands on land it
    // also takes.
    let mut registry = utils::ground_registry();
    registry.register_terrain("water");
    let dry = registry.register_domain(
        "dry",
        DomainDef::new(
            utils::GROUND,
            Terrains::Only(registry.terrain("land").unwrap().into()),
        ),
    );
    registry.register(
        EntityTypeDef::new("larva")
            .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
            .with_pool(Pool::health(20))
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(utils::standing("egg", utils::GROUND_DOMAIN).with_pool(Pool::health(60)));
    registry.register(
        EntityTypeDef::new("hatchling")
            .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
            .with_pool(Pool::health(30)),
    );

    registry.validate();
}

#[test]
fn validate_accepts_transition_through_form_of_same_footprint() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("larva", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_morphs([morph_through("egg", "hatchling")]),
    );
    registry.register(utils::standing("egg", utils::GROUND_DOMAIN).with_pool(Pool::health(60)));
    registry
        .register(utils::standing("hatchling", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));

    registry.validate();
}

#[test]
fn validate_accepts_transition_with_payable_costs() {
    let mut registry = utils::ground_registry();
    registry.register_resource("gold");
    registry.register(
        utils::standing("walker", utils::GROUND_DOMAIN)
            .with_movement(
                FixedU64::ONE,
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(100),
                utils::fixed("0.1"),
                FixedU64::ZERO,
                PoolInitial::Full,
            ))
            .with_morphs([MorphTransition::new(
                "flier",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(20),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                MorphReason::Change,
                vec![
                    Cost::Resources(price::from([("gold", 50)])),
                    Cost::Energy(FixedU64::from_num(20)),
                ],
                Vec::new(),
                [],
            )]),
    );
    registry.register(
        utils::standing("flier", utils::GROUND_DOMAIN).with_movement(
            FixedU64::ONE,
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        ),
    );

    registry.validate();
}

//
// ─── Turrets ──────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(
    expected = "entity type 'bunker' carries a turret that fires on the move but cannot move"
)]
fn register_rejects_turret_firing_while_moving_without_movement() {
    let mut registry = utils::ground_registry();
    let rolling = registry.register_turret(
        "rolling",
        TurretDef::new(
            Weapon::new(utils::GROUND, Delivery::Instant, None, Slain::Remains),
            TurretStats::default(),
            WeaponConduct::OnTheMove,
        ),
    );
    registry.register(
        utils::standing("bunker", utils::GROUND_DOMAIN)
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
fn register_rejects_turret_mounted_off_its_footprint() {
    let mut registry = utils::ground_registry();
    let gun = registry.register_turret(
        "gun",
        TurretDef::new(
            Weapon::new(utils::GROUND, Delivery::Instant, None, Slain::Remains),
            TurretStats::default(),
            WeaponConduct::Halts,
        ),
    );
    registry.register(
        utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
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
fn register_rejects_turret_reading_stat_its_body_lacks() {
    let mut registry = utils::ground_registry();
    let gun = registry.register_turret(
        "gun",
        TurretDef::new(
            Weapon::new(utils::GROUND, Delivery::Instant, None, Slain::Remains),
            TurretStats::default(),
            WeaponConduct::Halts,
        ),
    );
    registry.register(
        utils::standing("keep", utils::GROUND_DOMAIN).with_turrets([TurretMount::new(
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
        utils::standing("hive", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(300))
            .with_field_sources([emitter(creep)])
            .with_field_placement([FieldPlacement::Requires {
                field: creep,
                of: Affiliation::Anyone,
                coverage: FieldCoverage::Every,
            }])
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Any,
                vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![EntityModifier {
                        stat: EntityStatId::MAX_HEALTH,
                        op: ModifierOp::PercentAdd,
                        magnitude: FixedI64::ONE,
                    }],
                    pool_shift: PoolShift::Share,
                })],
                Vec::new(),
                None,
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
fn register_rejects_standing_act_on_foreign_field() {
    let mut registry = utils::ground_registry();
    // A handle minted by another registry, which this one never registered.
    let foreign = utils::ground_registry().register_field(
        "blight",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Never,
            FieldVision::Dark,
            Detection::Blind,
        ),
    );
    registry.register(
        utils::standing("pylon", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
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
        utils::standing("pylon", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
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
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Never,
            FieldVision::Watched,
            Detection::Blind,
        ),
    );
    let dark = registry.register_field(
        "power",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Blind,
        ),
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
#[should_panic(expected = "field 'creep' names unregistered terrains 0b10")]
fn register_rejects_field_on_unregistered_terrain() {
    utils::ground_registry().register_field(
        "creep",
        FieldDef::new(
            Terrains::Only(TerrainMask::from(TerrainId::new(2))),
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Blind,
        ),
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' projects an unregistered field")]
fn register_rejects_source_of_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register(
        utils::standing("hive", utils::GROUND_DOMAIN).with_field_sources([emitter(creep)]),
    );
}

#[test]
#[should_panic(expected = "entity type 'spore' reads an unregistered field for placement")]
fn register_rejects_placement_on_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register(
        utils::standing("spore", utils::GROUND_DOMAIN)
            .with_field_placement([FieldPlacement::Forbids { field: creep }]),
    );
}

#[test]
#[should_panic(expected = "entity type 'zergling' answers to an unregistered field")]
fn register_rejects_effect_of_foreign_field() {
    let mut foreign = utils::ground_registry();
    let creep = creep_field(&mut foreign);
    utils::ground_registry().register(
        utils::standing("zergling", utils::GROUND_DOMAIN).with_field_effects([FieldEffect::new(
            creep,
            Affiliation::Own,
            FieldCoverage::Every,
            Vec::new(),
            vec![EntityEffect::Disable],
            None,
        )]),
    );
}

#[test]
#[should_panic(expected = "decay cycle must be positive")]
fn field_rejects_zero_decay_cycle() {
    FieldDef::new(
        Terrains::Any,
        FieldDecay::Gradual { cycle: 0 },
        FieldVision::Dark,
        Detection::Blind,
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' starts a field beyond its radius")]
fn register_rejects_source_starting_beyond_radius() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", utils::GROUND_DOMAIN).with_field_sources([FieldSourceDef::new(
            creep,
            3,
            FieldGrowth::Gradual {
                cycle: 2,
                initial_radius: 5,
            },
            Emission::Nothing,
            Emission::Full,
        )]),
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' projects a field beyond its radius while rising")]
fn register_rejects_source_rising_beyond_radius() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", utils::GROUND_DOMAIN)
            .with_rise(4, [])
            .with_field_sources([FieldSourceDef::new(
                creep,
                3,
                FieldGrowth::Instant,
                Emission::Held(5),
                Emission::Full,
            )]),
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' projects a field while rising but never rises")]
fn register_rejects_source_rising_on_type_that_never_rises() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", utils::GROUND_DOMAIN).with_field_sources([FieldSourceDef::new(
            creep,
            3,
            FieldGrowth::Instant,
            Emission::Held(1),
            Emission::Full,
        )]),
    );
}

#[test]
#[should_panic(expected = "entity type 'hive' projects a field beyond its radius while disabled")]
fn register_rejects_source_holding_beyond_radius_while_disabled() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("hive", utils::GROUND_DOMAIN).with_field_sources([FieldSourceDef::new(
            creep,
            3,
            FieldGrowth::Instant,
            Emission::Nothing,
            Emission::Held(4),
        )]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'zergling' has a field effect with nothing inside or outside"
)]
fn register_rejects_field_effect_with_no_side() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", utils::GROUND_DOMAIN).with_field_effects([FieldEffect::new(
            creep,
            Affiliation::Anyone,
            FieldCoverage::Any,
            Vec::new(),
            Vec::new(),
            None,
        )]),
    );
}

#[test]
#[should_panic(expected = "entity type 'zergling' has a field effect with no modifiers")]
fn register_rejects_field_effect_with_no_modifiers() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", utils::GROUND_DOMAIN).with_field_effects([FieldEffect::new(
            creep,
            Affiliation::Anyone,
            FieldCoverage::Any,
            vec![EntityEffect::Modifiers(EntityModifiers::Stats(Vec::new()))],
            Vec::new(),
            None,
        )]),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'zergling' has a field effect on stat 'armor', which the type does not carry"
)]
fn register_rejects_field_effect_on_stat_type_lacks() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(35))
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Every,
                Vec::new(),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    EntityModifier {
                        stat: EntityStatId::ARMOR,
                        op: ModifierOp::FlatAdd,
                        magnitude: FixedI64::from_num(1),
                    },
                ]))],
                None,
            )]),
    );
}

#[test]
fn register_accepts_field_effect_on_rate_its_pool_carries() {
    let mut registry = utils::ground_registry();
    let creep = creep_field(&mut registry);
    registry.register(
        utils::standing("zergling", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(35))
            .with_field_effects([FieldEffect::new(
                creep,
                Affiliation::Anyone,
                FieldCoverage::Every,
                Vec::new(),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    EntityModifier {
                        stat: EntityStatId::HEALTH_DRAIN,
                        op: ModifierOp::FlatAdd,
                        magnitude: FixedI64::from_num(1),
                    },
                ]))],
                None,
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Watch {
                    at: Spot::Cell {
                        reach: Reach::Wherever,
                    },
                    radius: Quantity::Constant(3),
                    duration: Quantity::Constant(0),
                    detection: Detection::Blind,
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
#[should_panic(expected = "player-cast skill 'haste' requires something of an acting entity")]
fn register_rejects_player_cast_with_actor_leaf_under_node() {
    let mut registry = utils::ground_registry();
    let buff = haste_buff(&mut registry);
    let mut skill = player_cast(buff);
    // A node asks of the actor as soon as one leaf under it does, however
    // deep: the health line sits under an any inside an all.
    skill.requires = vec![Requirement::All(vec![
        Requirement::Tag("hall".to_string()),
        Requirement::Any(vec![
            Requirement::Tag("keep".to_string()),
            Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5")))),
        ]),
    ])];
    registry.register_skill("haste", skill);
}

#[test]
fn register_accepts_player_cast_with_player_leaves_under_node() {
    let mut registry = utils::ground_registry();
    let buff = haste_buff(&mut registry);
    let mut skill = player_cast(buff);
    skill.requires = vec![Requirement::Any(vec![
        Requirement::Tag("hall".to_string()),
        Requirement::Tag("keep".to_string()),
    ])];
    registry.register_skill("haste", skill);
}

#[test]
#[should_panic(
    expected = "entity type 'marine' requires 'runner' docked, which is not a registered annex"
)]
fn validate_rejects_annexed_requirement_naming_type_that_is_no_annex() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("runner", utils::GROUND_DOMAIN).with_pool(Pool::health(10)));
    registry.register(
        utils::standing("marine", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(10))
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Field {
                    at: Spot::Cell {
                        reach: Reach::Wherever,
                    },
                    field: creep,
                    radius: Quantity::Constant(2),
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
        utils::standing("corpse", utils::GROUND_DOMAIN)
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Fallen {
                        kinds: Kinds::Any,
                        reach: Reach::Wherever,
                    },
                    entity_type: corpse,
                    count: Quantity::Constant(2),
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Standing,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_remains_cast_with_summon_effect() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("skeleton", utils::GROUND_DOMAIN));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    registry.register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Fallen {
                        kinds: Kinds::Any,
                        reach: Reach::Wherever,
                    },
                    entity_type: skeleton,
                    count: Quantity::Constant(2),
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Standing,
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Enemy,
                            kinds: Kinds::types(["ghost"]),
                        },
                        reach: Reach::Wherever,
                    },
                },
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Heal {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Allied,
                            kinds: Kinds::types(["priest"]),
                        },
                        reach: Reach::Wherever,
                    },
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("priest", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(20))
            .with_skills([heal]),
    );

    registry.validate();
}

#[test]
fn register_accepts_summon_aimed_at_caster_and_at_cell() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("skeleton", utils::GROUND_DOMAIN));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    let summons = |at| SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                at,
                entity_type: skeleton,
                count: Quantity::Constant(1),
                landing: Landing::Anywhere,
                within: Vec::new(),
                arrives: SummonArrival::Standing,
            },
        },
        requires: Vec::new(),
    };
    registry.register_skill("guard_me", summons(SummonAt::Caster));
    registry.register_skill(
        "guard_there",
        summons(SummonAt::Cell {
            reach: Reach::Wherever,
        }),
    );
}

#[test]
#[should_panic(expected = "skill 'raise_dead' summons an unregistered entity type")]
fn register_rejects_summon_of_foreign_type() {
    let mut foreign = utils::ground_registry();
    foreign.register(utils::standing("skeleton", utils::GROUND_DOMAIN));
    let skeleton = foreign.type_id("skeleton").expect("just registered");
    utils::ground_registry().register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Fallen {
                        kinds: Kinds::Any,
                        reach: Reach::Wherever,
                    },
                    entity_type: skeleton,
                    count: Quantity::Constant(1),
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Standing,
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
    registry.register(utils::standing("skeleton", utils::GROUND_DOMAIN));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    registry.register_skill(
        "raise_dead",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Fallen {
                        kinds: Kinds::Any,
                        reach: Reach::Wherever,
                    },
                    entity_type: skeleton,
                    count: Quantity::Constant(0),
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Standing,
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Enemy,
                            kinds: Kinds::Any,
                        },
                        reach: Reach::Within(Quantity::Stat(raise_range)),
                    },
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("necromancer", utils::GROUND_DOMAIN)
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Enemy,
                            kinds: Kinds::Any,
                        },
                        reach: Reach::Within(Quantity::Stat(raise_range)),
                    },
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(utils::standing("necromancer", utils::GROUND_DOMAIN).with_skills([bolt]));
}

#[test]
#[should_panic(
    expected = "entity type 'observatory' has skill 'scan' reading its watch radius from a stat it does not carry"
)]
fn register_rejects_watch_radius_by_stat_caster_lacks() {
    let mut registry = utils::ground_registry();
    let scan_radius = registry.register_entity_stat("scan_radius", FixedU64::ZERO);
    let scan = registry.register_skill(
        "scan",
        watching(Quantity::Stat(scan_radius), Quantity::Constant(40)),
    );
    registry.register(utils::standing("observatory", utils::GROUND_DOMAIN).with_skills([scan]));
}

#[test]
#[should_panic(expected = "skill 'scan' watches for no time at all")]
fn register_rejects_watch_duration_by_stat_with_zero_floor() {
    let mut registry = utils::ground_registry();
    let scan_time = registry.register_entity_stat("scan_time", FixedU64::ZERO);
    registry.register_skill(
        "scan",
        watching(Quantity::Constant(6), Quantity::Stat(scan_time)),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'necromancer' has skill 'raise_host' reading its summon count from a stat it does not carry"
)]
fn register_rejects_summon_count_by_stat_caster_lacks() {
    let mut registry = utils::ground_registry();
    let host_size = registry.register_entity_stat("host_size", FixedU64::ONE);
    let raise_host = summoning(&mut registry, host_size);
    let raise_host = registry.register_skill("raise_host", raise_host);
    registry
        .register(utils::standing("necromancer", utils::GROUND_DOMAIN).with_skills([raise_host]));
}

#[test]
#[should_panic(expected = "skill 'raise_host' summons nothing at all")]
fn register_rejects_summon_count_by_stat_with_floor_under_one() {
    let mut registry = utils::ground_registry();
    // A floor of one half truncates to no skeleton at all.
    let host_size = registry.register_entity_stat("host_size", FixedU64::lit("0.5"));
    let raise_host = summoning(&mut registry, host_size);
    registry.register_skill("raise_host", raise_host);
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
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Constant(20),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Enemy,
                            kinds: Kinds::Any,
                        },
                        reach: Reach::Wherever,
                    },
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(utils::standing("necromancer", utils::GROUND_DOMAIN).with_skills([bolt]));
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
                casting: Casting::Delayed {
                    point: Quantity::Constant(0),
                    period: Quantity::Constant(10),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Caster,
                },
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
                casting: Casting::Delayed {
                    point: Quantity::Constant(10),
                    period: Quantity::Stat(recovery),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Caster,
                },
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
                casting: Casting::Delayed {
                    point: Quantity::Constant(10),
                    period: Quantity::Constant(5),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Caster,
                },
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
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Stat(ritual_time),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Caster,
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
fn register_accepts_cast_point_from_stat_floored_under_one_tick() {
    let mut registry = utils::ground_registry();
    // A floor of 0.5 is read as one tick, rounded up: never no time at all.
    let ritual_time = registry.register_entity_stat("ritual_time", utils::fixed("0.5"));
    registry.register_skill(
        "bolt",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Stat(ritual_time),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Caster,
                },
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
                casting: Casting::Delayed {
                    point: Quantity::Stat(ritual_time),
                    period: Quantity::Stat(ritual_time),
                    pays: PaymentPoint::AtLanding,
                },
                effect: EntityCastEffect::Damage {
                    amount: FixedU64::ONE,
                    on: Recipient::Standing {
                        whom: Whom {
                            side: Affiliation::Enemy,
                            kinds: Kinds::Any,
                        },
                        reach: Reach::Wherever,
                    },
                },
            },
            requires: Vec::new(),
        },
    );
    registry.register(
        utils::standing("necromancer", utils::GROUND_DOMAIN)
            .with_stat(ritual_time, FixedU64::from_num(20))
            .with_skills([bolt]),
    );
}

#[test]
fn register_accepts_weapon_beside_carrier() {
    let mut registry = utils::ground_registry();
    registry.register_resource("wood");
    registry.register(
        utils::standing("tree", utils::GROUND_DOMAIN)
            .with_resource_source("wood", DepletionPolicy::Destroy),
    );
    registry.register(
        utils::standing("ghoul", utils::GROUND_DOMAIN)
            .with_attack_def(utils::GROUND, Delivery::Instant, None, Slain::Remains)
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
                casting: Casting::Instant,
                effect: EntityCastEffect::Field {
                    at: Spot::Cell {
                        reach: Reach::Wherever,
                    },
                    field: creep,
                    radius: Quantity::Constant(2),
                    action: FieldAction::Cover,
                },
            },
            requires: Vec::new(),
        },
    );
}

//
// ─── Concealment and detection ────────────────────────────────────────────────
//

#[test]
fn register_keeps_type_concealment() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("shade", utils::GROUND_DOMAIN).with_concealment(Concealment::Concealed),
    );
    registry.register(utils::standing("footman", utils::GROUND_DOMAIN));
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
fn register_accepts_buff_that_only_conceals() {
    let mut registry = utils::ground_registry();
    let cloak = registry.register_entity_buff("cloak", concealing(Lasting::Forever));
    assert_eq!(
        registry.entity_buff_def(cloak).effects,
        vec![EntityEffect::Conceal]
    );
}

#[test]
fn register_accepts_buff_that_only_names_interruption() {
    let mut registry = utils::ground_registry();
    let marked = registry.register_entity_buff(
        "marked",
        EntityBuffDef {
            effects: Vec::new(),
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: vec![Interruption::Attack],
        },
    );
    assert_eq!(
        registry.entity_buff_def(marked).interrupted_by,
        vec![Interruption::Attack]
    );
}

#[test]
#[should_panic(expected = "entity buff 'cloak' lasts for no time at all")]
fn register_rejects_buff_lasting_no_time() {
    utils::ground_registry().register_entity_buff("cloak", concealing(Lasting::For(0)));
}

#[test]
#[should_panic(expected = "entity buff 'cloak' has an upkeep that costs nothing")]
fn register_rejects_upkeep_costing_nothing() {
    utils::ground_registry().register_entity_buff(
        "cloak",
        concealing(Lasting::Upkeep {
            costs: Vec::new(),
            period: 1,
        }),
    );
}

#[test]
#[should_panic(expected = "entity buff 'cloak' pays its upkeep every zero ticks")]
fn register_rejects_upkeep_paid_every_zero_ticks() {
    utils::ground_registry().register_entity_buff(
        "cloak",
        concealing(Lasting::Upkeep {
            costs: vec![Cost::Energy(FixedU64::ONE)],
            period: 0,
        }),
    );
}

#[test]
#[should_panic(expected = "entity buff 'cloak' costs unregistered resource kind 'gas'")]
fn register_rejects_upkeep_in_unregistered_resource() {
    utils::ground_registry().register_entity_buff(
        "cloak",
        concealing(Lasting::Upkeep {
            costs: vec![Cost::Resources(price::from([("gas", 1)]))],
            period: 20,
        }),
    );
}

#[test]
#[should_panic(expected = "entity buff 'nothing' does nothing at all")]
fn register_rejects_buff_doing_nothing() {
    utils::ground_registry().register_entity_buff(
        "nothing",
        EntityBuffDef {
            effects: Vec::new(),
            lasting: Lasting::For(10),
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
}

#[test]
fn register_keeps_field_detection() {
    let mut registry = utils::ground_registry();
    let true_sight = registry.register_field(
        "true_sight",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Reveals(utils::GROUND.into()),
        ),
    );
    assert_eq!(
        registry.field_def(true_sight).detection(),
        Detection::Reveals(utils::GROUND.into())
    );
}

#[test]
#[should_panic(expected = "a field on no terrain lies nowhere; declare it on \"any\"")]
fn field_on_no_terrain_panics() {
    FieldDef::new(
        Terrains::Only(TerrainMask::EMPTY),
        FieldDecay::Instant,
        FieldVision::Dark,
        Detection::Blind,
    );
}

#[test]
#[should_panic(expected = "a field revealing no layer detects nothing; declare it blind")]
fn field_rejects_detection_revealing_no_layer() {
    FieldDef::new(
        Terrains::Any,
        FieldDecay::Instant,
        FieldVision::Dark,
        Detection::Reveals(LayerMask::EMPTY),
    );
}

#[test]
#[should_panic(expected = "field 'sonar' detects on unregistered layers")]
fn register_rejects_field_detecting_unregistered_layer() {
    utils::ground_registry().register_field(
        "sonar",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Reveals(utils::WATER.into()),
        ),
    );
}

#[test]
#[should_panic(expected = "skill 'scan' detects on unregistered layers")]
fn register_rejects_watch_detecting_unregistered_layer() {
    utils::ground_registry().register_skill(
        "scan",
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Watch {
                    at: Spot::Cell {
                        reach: Reach::Wherever,
                    },
                    radius: Quantity::Constant(3),
                    duration: Quantity::Constant(5),
                    detection: Detection::Reveals(utils::WATER.into()),
                },
            },
            requires: Vec::new(),
        },
    );
}

#[test]
#[should_panic(expected = "entity buff 'hollow' has an effect with no modifiers")]
fn register_rejects_buff_effect_with_no_modifiers() {
    let mut registry = utils::ground_registry();
    registry.register_entity_buff(
        "hollow",
        EntityBuffDef {
            effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(Vec::new()))],
            lasting: Lasting::Forever,
            stack_rule: StackRule::Ignore,
            interrupted_by: Vec::new(),
        },
    );
}

#[test]
#[should_panic(
    expected = "entity type 'ghost' has skill 'bleed' keeping up a buff from health but no health pool"
)]
fn register_rejects_self_cast_health_upkeep_on_type_without_health() {
    let mut registry = utils::ground_registry();
    let bleeding = registry.register_entity_buff(
        "bleeding",
        concealing(Lasting::Upkeep {
            costs: vec![Cost::Health(FixedU64::ONE)],
            period: 1,
        }),
    );
    let skill = registry.register_skill("bleed", self_cast(bleeding));
    registry.register(utils::standing("ghost", utils::GROUND_DOMAIN).with_skills([skill]));
}

#[test]
#[should_panic(
    expected = "entity type 'ghost' has skill 'cloak' keeping up a buff from energy but no energy pool"
)]
fn register_rejects_self_cast_upkeep_from_pool_type_lacks() {
    let mut registry = utils::ground_registry();
    let cloak = registry.register_entity_buff(
        "cloak",
        concealing(Lasting::Upkeep {
            costs: vec![Cost::Energy(FixedU64::ONE)],
            period: 1,
        }),
    );
    let skill = registry.register_skill("cloak", self_cast(cloak));
    registry.register(
        utils::standing("ghost", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(40))
            .with_skills([skill]),
    );
}

#[test]
fn register_accepts_self_cast_upkeep_from_pool_type_carries() {
    let mut registry = utils::ground_registry();
    let cloak = registry.register_entity_buff(
        "cloak",
        concealing(Lasting::Upkeep {
            costs: vec![Cost::Energy(FixedU64::ONE)],
            period: 1,
        }),
    );
    let skill = registry.register_skill("cloak", self_cast(cloak));
    registry.register(
        utils::standing("ghost", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(40))
            .with_pool(Pool::energy(50))
            .with_skills([skill]),
    );
    registry.validate();
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
    registry.register(utils::standing("grub", utils::GROUND_DOMAIN).with_pool(Pool::health(25)));
    registry.validate();
}

#[test]
#[should_panic(
    expected = "entity type 'hatch' breeding 'grub' seats it in berth group 'brood', which the breeder does not declare"
)]
fn validate_rejects_berthed_broodling_in_group_breeder_lacks() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::sized("hatch", utils::GROUND_DOMAIN, CellSize::new(3, 3))
            .with_pool(Pool::health(300))
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
        utils::sized("grub", utils::GROUND_DOMAIN, CellSize::new(2, 2))
            .with_pool(Pool::health(25))
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
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Passable,
                utils::GROUND,
            )
            .with_pool(Pool::health(25))
            .with_morphs([morph_through("egg", "worker")]),
    );
    registry.register(utils::standing("egg", utils::GROUND_DOMAIN).with_pool(Pool::health(100)));
    registry.register(utils::standing("worker", utils::GROUND_DOMAIN).with_pool(Pool::health(30)));
    registry.validate();
}

//
// ─── Pools ────────────────────────────────────────────────────────────────────
//

#[test]
fn registered_pool_writes_its_stats_with_unnamed_rates_at_zero() {
    let mut registry = utils::ground_registry();
    registry.register(
        utils::standing("well", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(100))
            .with_pool(Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(40),
                FixedU64::from_num(2),
                FixedU64::ZERO,
                PoolInitial::Full,
            )),
    );
    let well = registry.entity("well").unwrap();

    // Health named its maximum alone; energy its maximum and regen.
    assert_eq!(
        well.base_stat(EntityStatId::MAX_HEALTH),
        Some(FixedU64::from_num(100))
    );
    assert_eq!(
        well.base_stat(EntityStatId::HEALTH_REGEN),
        Some(FixedU64::ZERO)
    );
    assert_eq!(
        well.base_stat(EntityStatId::HEALTH_DRAIN),
        Some(FixedU64::ZERO)
    );
    assert_eq!(
        well.base_stat(EntityStatId::MAX_ENERGY),
        Some(FixedU64::from_num(40))
    );
    assert_eq!(
        well.base_stat(EntityStatId::ENERGY_REGEN),
        Some(FixedU64::from_num(2))
    );
    assert_eq!(
        well.base_stat(EntityStatId::ENERGY_DRAIN),
        Some(FixedU64::ZERO)
    );
}

#[test]
fn type_without_pool_carries_none_of_its_stats() {
    let mut registry = utils::ground_registry();
    registry.register(utils::standing("rock", utils::GROUND_DOMAIN));
    let rock = registry.entity("rock").unwrap();

    for stat in [
        EntityStatId::MAX_HEALTH,
        EntityStatId::HEALTH_REGEN,
        EntityStatId::HEALTH_DRAIN,
        EntityStatId::MAX_ENERGY,
        EntityStatId::ENERGY_REGEN,
        EntityStatId::ENERGY_DRAIN,
    ] {
        assert_eq!(rock.base_stat(stat), None, "{stat:?}");
    }
}

#[test]
fn pool_role_names_each_stat_of_built_in_pools() {
    let registry = utils::ground_registry();

    for (stat, role) in [
        (
            EntityStatId::MAX_HEALTH,
            (PoolId::HEALTH, PoolRole::Maximum),
        ),
        (
            EntityStatId::HEALTH_REGEN,
            (PoolId::HEALTH, PoolRole::Regen),
        ),
        (
            EntityStatId::HEALTH_DRAIN,
            (PoolId::HEALTH, PoolRole::Drain),
        ),
        (
            EntityStatId::MAX_ENERGY,
            (PoolId::ENERGY, PoolRole::Maximum),
        ),
        (
            EntityStatId::ENERGY_REGEN,
            (PoolId::ENERGY, PoolRole::Regen),
        ),
        (
            EntityStatId::ENERGY_DRAIN,
            (PoolId::ENERGY, PoolRole::Drain),
        ),
    ] {
        assert_eq!(registry.pool_role_of(stat), Some(role), "{stat:?}");
    }
    assert_eq!(registry.pool_role_of(EntityStatId::ARMOR), None);
}

//
// ─── Relocation ───────────────────────────────────────────────────────────────
//

#[test]
fn register_accepts_every_journey() {
    let mut registry = utils::ground_registry();
    let target = || AimParty::Target {
        whom: own_anything(),
        reach: Reach::Wherever,
    };
    let around = |count| AimParty::Around {
        at: cell(),
        gathering: gathering(count),
    };
    let skills = [
        (
            "blink",
            from_caster(CasterParty::Caster, FromCasterTo::Aim(cell())),
        ),
        (
            "portal",
            from_caster(
                CasterParty::CasterAndAround(gathering(Count::AtMost(Quantity::Constant(24)))),
                FromCasterTo::Aim(Away::Standing {
                    whom: own_anything(),
                    reach: Reach::Wherever,
                }),
            ),
        ),
        (
            "send_off",
            from_caster(
                CasterParty::Around(gathering(Count::All)),
                FromCasterTo::Aim(cell()),
            ),
        ),
        (
            "homeward",
            from_caster(CasterParty::Caster, FromCasterTo::Home(Kinds::Any)),
        ),
        ("pull", from_aim(target(), FromAimTo::Caster)),
        ("recall", from_aim(around(Count::All), FromAimTo::Caster)),
        (
            "evacuate",
            from_aim(around(Count::All), FromAimTo::Home(Kinds::Any)),
        ),
        ("send_home", from_aim(target(), FromAimTo::Home(Kinds::Any))),
        (
            "swap",
            Journey::Swap {
                whom: own_anything(),
                reach: Reach::Within(Quantity::Constant(10)),
            },
        ),
    ];
    for (name, journey) in skills {
        registry.register_skill(name, relocating(journey));
    }
    registry.validate();
}

#[test]
#[should_panic(expected = "skill 'recall' relocates nobody at all")]
fn register_rejects_relocation_of_nobody() {
    utils::ground_registry().register_skill(
        "recall",
        relocating(from_aim(
            AimParty::Around {
                at: cell(),
                gathering: gathering(Count::AtMost(Quantity::Constant(0))),
            },
            FromAimTo::Caster,
        )),
    );
}

#[test]
#[should_panic(expected = "skill 'homeward' relocates nobody at all")]
fn register_rejects_home_journey_of_nobody() {
    utils::ground_registry().register_skill(
        "homeward",
        relocating(from_caster(
            CasterParty::Around(gathering(Count::AtMost(Quantity::Constant(0)))),
            FromCasterTo::Home(Kinds::Any),
        )),
    );
}

#[test]
#[should_panic(expected = "skill 'recall' relocates nobody at all")]
fn register_rejects_relocation_count_by_stat_with_zero_floor() {
    let mut registry = utils::ground_registry();
    let host_size = registry.register_entity_stat("host_size", FixedU64::ZERO);
    registry.register_skill(
        "recall",
        relocating(from_aim(
            AimParty::Around {
                at: cell(),
                gathering: gathering(Count::AtMost(Quantity::Stat(host_size))),
            },
            FromAimTo::Caster,
        )),
    );
}

#[test]
#[should_panic(
    expected = "entity type 'recaller' has skill 'recall' reading its gathering radius from a stat it does not carry"
)]
fn register_rejects_gathering_radius_by_stat_caster_lacks() {
    let mut registry = utils::ground_registry();
    let recall_radius = registry.register_entity_stat("recall_radius", FixedU64::ZERO);
    let mut taken = gathering(Count::All);
    taken.radius = Quantity::Stat(recall_radius);
    let recall = registry.register_skill(
        "recall",
        relocating(from_aim(
            AimParty::Around {
                at: cell(),
                gathering: taken,
            },
            FromAimTo::Caster,
        )),
    );
    registry.register(utils::standing("recaller", utils::GROUND_DOMAIN).with_skills([recall]));
}

#[test]
#[should_panic(
    expected = "skill 'evacuate' relocation gathers entity type 'ghost', which is not registered"
)]
fn validate_rejects_relocation_gathering_unregistered_type() {
    let mut registry = utils::ground_registry();
    let mut taken = gathering(Count::All);
    taken.kinds = Kinds::only(vec![Kind::Type("ghost".into())]);
    registry.register_skill(
        "evacuate",
        relocating(from_aim(
            AimParty::Around {
                at: cell(),
                gathering: taken,
            },
            FromAimTo::Home(Kinds::Any),
        )),
    );
    registry.validate();
}

#[test]
#[should_panic(
    expected = "skill 'homeward' relocation goes home to entity type 'ghost', which is not registered"
)]
fn validate_rejects_home_of_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "homeward",
        relocating(from_caster(
            CasterParty::Caster,
            FromCasterTo::Home(Kinds::only(vec![Kind::Type("ghost".into())])),
        )),
    );
    registry.validate();
}

#[test]
#[should_panic(expected = "skill 'swap' aims at entity type 'ghost', which is not registered")]
fn validate_rejects_swap_aimed_at_unregistered_type() {
    let mut registry = utils::ground_registry();
    registry.register_skill(
        "swap",
        relocating(Journey::Swap {
            whom: Whom {
                side: Affiliation::Own,
                kinds: Kinds::only(vec![Kind::Type("ghost".into())]),
            },
            reach: Reach::Wherever,
        }),
    );
    registry.validate();
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A type that lies where it is left and is gone after `decay` ticks.
fn remains(name: &str, decay: u32) -> EntityTypeDef {
    utils::standing(name, utils::GROUND_DOMAIN)
        .with_tags(["remains"])
        .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(decay))
}

/// A 2x2 primary that raises the `annexes` it docks at (2, 0), standing on the
/// site to do it.
fn primary(annexes: [&str; 1]) -> EntityTypeDef {
    utils::sized("keep", utils::GROUND_DOMAIN, CellSize::new(2, 2))
        .with_pool(Pool::health(100))
        .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
        .with_builder(
            annexes,
            BuilderAttendance::Crew(WorkPresence::Present {
                crew: CrewLimit::ONE,
            }),
        )
        .with_docks([(CellPos::new(2, 0), Kinds::types(annexes))])
}

/// An annex of `size` that rises, endures alone and is bound to its owner.
fn annex(name: &str, size: CellSize) -> EntityTypeDef {
    utils::sized(name, utils::GROUND_DOMAIN, size)
        .with_pool(Pool::health(10))
        .with_rise(4, utils::rise_initial_pools(&[PoolId::HEALTH]))
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
            price: Price::new(),
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
            entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                stat: EntityStatId::SPEED,
                op: ModifierOp::PercentAdd,
                magnitude: FixedI64::ONE,
            }])],
            duration: Some(10),
            stack_rule: StackRule::Refresh,
        },
    )
}

/// A 3×3 breeder of `grub`s with a brood group of `slots` seats and a limit of
/// `limit`.
fn hatch(name: &str, slots: usize, limit: usize) -> EntityTypeDef {
    utils::sized(name, utils::GROUND_DOMAIN, CellSize::new(3, 3))
        .with_pool(Pool::health(300))
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
        .with_location(
            utils::GROUND_DOMAIN,
            CellSize::ONE,
            Solidity::Passable,
            utils::GROUND,
        )
        .with_pool(Pool::health(25))
        .with_broodling(Attachment::new("brood", BerthStance::Still))
}

/// A free, timed, committed transition into `into` worn as `via` on the way.
fn morph_through(via: &str, into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::via((via).to_string(), [], ViaInterrupted::reverts([])),
        Quantity::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [],
    )
}

/// A free, timed, committed transition into the named type.
fn morph_into(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::direct(MorphInterrupted::Reverts),
        Quantity::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [],
    )
}

/// A free, timed, committed transition into the named type on `course`,
/// carrying `land_pool_carry` into it.
fn carrying_into(
    into: &str,
    course: MorphCourse,
    land_pool_carry: impl IntoIterator<Item = (PoolId, PoolCarry)>,
) -> MorphTransition {
    MorphTransition::new(
        into,
        course,
        Quantity::Constant(20),
        MorphPlacement::Revalidate,
        MorphCancel::Committed,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        land_pool_carry,
    )
}

/// A buff that conceals its carrier and nothing else, lasting as given.
fn concealing(lasting: Lasting) -> EntityBuffDef {
    EntityBuffDef {
        effects: vec![EntityEffect::Conceal],
        lasting,
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// A free, instant skill an entity casts to put `buff` on itself.
fn self_cast(buff: EntityBuffId) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::ApplyBuff {
                buff,
                on: Recipient::Caster,
            },
        },
        requires: Vec::new(),
    }
}

/// A gradually receding ground field named "creep", registered into `registry`.
fn creep_field(registry: &mut ContentRegistry) -> FieldId {
    registry.register_field(
        "creep",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Gradual { cycle: 4 },
            FieldVision::Dark,
            Detection::Blind,
        ),
    )
}

/// An instant emitter of `field` reaching three cells.
fn emitter(field: FieldId) -> FieldSourceDef {
    FieldSourceDef::new(
        field,
        3,
        FieldGrowth::Instant,
        Emission::Nothing,
        Emission::Full,
    )
}

/// A buff held on an amount of health, moving `max_health` by `magnitude` as
/// a percentage under `pool_shift`.
fn held_on_health_buff(
    magnitude: &str,
    pool_shift: PoolShift,
    threshold: Threshold,
) -> EntityBuffDef {
    EntityBuffDef {
        effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
            modifiers: vec![EntityModifier {
                stat: EntityStatId::MAX_HEALTH,
                op: ModifierOp::PercentAdd,
                magnitude: magnitude.parse().expect("a percentage is a value"),
            }],
            pool_shift,
        })],
        lasting: Lasting::While(Requirement::Health(Bound::Amount(threshold))),
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// A buff held while `requirement` is met, moving `stat` by a flat
/// `magnitude` under `pool_shift`.
fn held_on_amount_buff(
    requirement: Requirement,
    stat: EntityStatId,
    magnitude: &str,
    pool_shift: PoolShift,
) -> EntityBuffDef {
    EntityBuffDef {
        effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
            modifiers: vec![EntityModifier {
                stat,
                op: ModifierOp::FlatAdd,
                magnitude: magnitude.parse().expect("a magnitude is a value"),
            }],
            pool_shift,
        })],
        lasting: Lasting::While(requirement),
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// A buff held while `judged` reads at least 1, raising `moved` by 1.
fn held_on_stat_buff(judged: EntityStatId, moved: EntityStatId) -> EntityBuffDef {
    EntityBuffDef {
        effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
            EntityModifier {
                stat: moved,
                op: ModifierOp::FlatAdd,
                magnitude: FixedI64::ONE,
            },
        ]))],
        lasting: Lasting::While(Requirement::Stat {
            stat: judged,
            bound: Bound::Amount(Threshold::AtLeast(FixedU64::ONE)),
        }),
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// A health pool of 100 starting at `initial`.
fn health_starting(initial: PoolInitial) -> Pool {
    Pool::builtin(
        PoolId::HEALTH,
        FixedU64::from_num(100),
        FixedU64::ZERO,
        FixedU64::ZERO,
        initial,
    )
}

/// A free, instant entity skill that watches `radius` cells around an aimed
/// cell for `duration` ticks.
fn watching(radius: Quantity, duration: Quantity) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Watch {
                at: Spot::Cell {
                    reach: Reach::Wherever,
                },
                radius,
                duration,
                detection: Detection::Blind,
            },
        },
        requires: Vec::new(),
    }
}

/// A free, instant entity skill that raises as many `skeleton`s, registered
/// here, as the caster's `count` stat says, around itself.
fn summoning(registry: &mut ContentRegistry, count: EntityStatId) -> SkillDef {
    registry.register(utils::standing("skeleton", utils::GROUND_DOMAIN));
    let skeleton = registry.type_id("skeleton").expect("just registered");
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                at: SummonAt::Caster,
                entity_type: skeleton,
                count: Quantity::Stat(count),
                landing: Landing::Anywhere,
                within: Vec::new(),
                arrives: SummonArrival::Standing,
            },
        },
        requires: Vec::new(),
    }
}

/// A free, instant entity skill that relocates on `journey`, landing
/// anywhere.
fn relocating(journey: Journey) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Relocate {
                journey: Box::new(journey),
                landing: Landing::Anywhere,
            },
        },
        requires: Vec::new(),
    }
}

/// Every own body within 2 cells, up to `count`, the rest left standing.
fn gathering(count: Count) -> Gathering {
    Gathering {
        radius: Quantity::Constant(2),
        side: Affiliation::Own,
        kinds: Kinds::Any,
        sight: GatherSight::Blind,
        count,
        shortfall: Shortfall::Leave,
    }
}

/// `party` gathered at the caster, going `to`.
fn from_caster(party: CasterParty, to: FromCasterTo) -> Journey {
    Journey::FromCaster { party, to }
}

/// `party` gathered at the aim, going `to`.
fn from_aim(party: AimParty, to: FromAimTo) -> Journey {
    Journey::FromAim { party, to }
}

/// Something own standing, of any kind.
fn own_anything() -> Whom {
    Whom {
        side: Affiliation::Own,
        kinds: Kinds::Any,
    }
}

/// An aimed cell, cast from wherever the caster stands.
fn cell() -> Away {
    Away::Cell {
        reach: Reach::Wherever,
    }
}

/// A `warren` network of `capacity` letting in anyone of its owner's and
/// killing whoever is inside with its last gateway.
fn warren_def(capacity: u32) -> NetworkDef {
    NetworkDef::new(
        Kinds::Any,
        Affiliation::Own,
        capacity,
        PassengerFate::Destroy,
    )
}

/// A tunnel of `passage`, letting in anyone of its owner's, its survivor
/// standing.
fn tunnel_def(passage: Passage) -> TunnelDef {
    TunnelDef::new(Kinds::Any, Affiliation::Own, OtherEndLost::Stands, passage)
}

/// A held passage of `capacity` that kills whoever is inside with the
/// tunnel's last end.
fn held(capacity: u32) -> Passage {
    Passage::Held {
        capacity,
        fate: PassengerFate::Destroy,
    }
}

/// A one-cell gateway of what `joins` names with `doors`, declaring no
/// transport stat.
fn gateway(name: &str, joins: Joins, doors: Doors) -> EntityTypeDef {
    utils::standing(name, utils::GROUND_DOMAIN).with_gateway(joins, doors)
}

/// A free instant skill summoning `count` of `entity_type` at any cell.
fn summoning_end(entity_type: EntityTypeId, count: u32) -> SkillDef {
    SkillDef {
        cooldown: 1,
        caster: SkillCaster::Entity {
            costs: Vec::new(),
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                at: SummonAt::Cell {
                    reach: Reach::Wherever,
                },
                entity_type,
                count: Quantity::Constant(count),
                landing: Landing::Anywhere,
                within: Vec::new(),
                arrives: SummonArrival::Standing,
            },
        },
        requires: Vec::new(),
    }
}

/// `def` with the boarding pair of stats.
fn entering(def: EntityTypeDef) -> EntityTypeDef {
    def.with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
        .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
}

/// `def` with the unloading pair of stats.
fn leaving(def: EntityTypeDef) -> EntityTypeDef {
    def.with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
        .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
}

/// Registers a `power` field on the ground layer.
fn register_power(registry: &mut ContentRegistry) -> FieldId {
    registry.register_field(
        "power",
        FieldDef::new(
            Terrains::Any,
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Blind,
        ),
    )
}

/// A rule that every footprint cell be covered by its owner's `power`.
fn powered(power: FieldId) -> FieldPlacement {
    FieldPlacement::Requires {
        field: power,
        of: Affiliation::Own,
        coverage: FieldCoverage::Every,
    }
}

/// A one-cell `recruit` that trains in ten ticks and rises in ten.
fn rising_recruit() -> EntityTypeDef {
    utils::standing("recruit", utils::GROUND_DOMAIN)
        .with_train_time(10)
        .with_rise(10, Vec::new())
}

/// Registers a skill named `name` summoning one `type_name` rising beside the
/// caster, inside the `within` field rules.
fn register_summon(
    registry: &mut ContentRegistry,
    name: &str,
    type_name: &str,
    within: Vec<FieldPlacement>,
) {
    let entity_type = registry
        .type_id(type_name)
        .expect("the summoned type is registered");
    registry.register_skill(
        name,
        SkillDef {
            cooldown: 1,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Caster,
                    entity_type,
                    count: Quantity::Constant(1),
                    landing: Landing::Anywhere,
                    within,
                    arrives: SummonArrival::Rising,
                },
            },
            requires: Vec::new(),
        },
    );
}
