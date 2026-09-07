//! Content validation: every invalid [`EntityTypeDef`] must panic at
//! construction, not misbehave at runtime.

mod utils;

use ferrets_content::{
    berths::{BerthGroup, BerthsDef},
    build::BuilderAttendance,
    dying::DyingDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    resource::{Banking, DepletionPolicy, HarvestData},
    work::{Attachment, BerthStance, WorkPresence},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::layer_mask::LayerMask;
use utils::GROUND;

//
// ─── Happy path ───────────────────────────────────────────────────────────────
//

#[test]
fn fully_loaded_definition_is_valid() {
    let def = utils::standing("factotum", GROUND)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_health(50)
        .with_dying(3, None)
        .with_attack(utils::weapon(GROUND), 10, 1, 1, 4, 2)
        .with_cost([("gold", 30), ("wood", 10)])
        .with_train_time(4)
        .with_build_time(6)
        .with_trainer(["footman"])
        .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
        .with_builder(["depot"], BuilderAttendance::Crew(WorkPresence::Hidden))
        .with_resource_source("gold", DepletionPolicy::Destroy)
        .with_resource_carrier([(
            "gold",
            HarvestData::new(5, 5, 2, WorkPresence::Hidden, Banking::Carried),
        )])
        .with_resource_storage(["gold"]);

    assert_eq!(def.name, "factotum");
}

//
// ─── Identity and footprint ───────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "name must not be empty")]
fn empty_name_panics() {
    EntityTypeDef::new("");
}

#[test]
#[should_panic(expected = "occupation must not be empty")]
fn empty_occupation_panics() {
    EntityTypeDef::new("footman").with_location(LayerMask::EMPTY, CellSize::ONE, Solidity::Solid);
}

#[test]
#[should_panic(expected = "size dimensions must be greater than 0")]
fn zero_footprint_panics() {
    EntityTypeDef::new("footman").with_location(GROUND, CellSize::new(0, 2), Solidity::Solid);
}

//
// ─── Dying phase ──────────────────────────────────────────────────────────────
//
// Stat invariants (positive health/speed/attack_period/damage_point, weapon completeness)
// are validated at registration, not construction — see the registry tests.
//

#[test]
#[should_panic(expected = "dying_time must be greater than 0")]
fn zero_dying_time_panics() {
    DyingDef::new(0, None);
}

#[test]
#[should_panic(expected = "corpse_type must not be empty")]
fn empty_corpse_type_panics() {
    DyingDef::new(3, Some(""));
}

//
// ─── Cost and production ──────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "cost resource kinds must not be empty")]
fn empty_cost_kind_panics() {
    footman().with_cost([("", 30)]);
}

#[test]
#[should_panic(expected = "cost amounts must be greater than 0")]
fn zero_cost_amount_panics() {
    footman().with_cost([("gold", 0)]);
}

#[test]
#[should_panic(expected = "train_time must be greater than 0")]
fn zero_train_time_panics() {
    footman().with_train_time(0);
}

#[test]
#[should_panic(expected = "build_time must be greater than 0")]
fn zero_build_time_panics() {
    footman().with_build_time(0);
}

#[test]
#[should_panic(expected = "trains must not be empty")]
fn empty_trains_list_panics() {
    footman().with_trainer(Vec::<String>::new());
}

#[test]
#[should_panic(expected = "trained type names must not be empty")]
fn empty_trains_entry_panics() {
    footman().with_trainer(["footman", ""]);
}

#[test]
#[should_panic(expected = "builds must not be empty")]
fn empty_builds_list_panics() {
    footman().with_builder(
        Vec::<String>::new(),
        BuilderAttendance::Crew(WorkPresence::Hidden),
    );
}

#[test]
#[should_panic(expected = "constructed type names must not be empty")]
fn empty_builds_entry_panics() {
    footman().with_builder([""], BuilderAttendance::Crew(WorkPresence::Hidden));
}

//
// ─── Resources ────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "kind must not be empty")]
fn empty_source_kind_panics() {
    footman().with_resource_source("", DepletionPolicy::Destroy);
}

#[test]
#[should_panic(expected = "harvest_time must be greater than 0")]
fn zero_harvest_time_panics() {
    HarvestData::new(5, 5, 0, WorkPresence::Present, Banking::Carried);
}

#[test]
#[should_panic(expected = "capacity must be greater than 0")]
fn zero_carry_capacity_panics() {
    HarvestData::new(0, 0, 2, WorkPresence::Present, Banking::Carried);
}

#[test]
#[should_panic(expected = "carries must not be empty")]
fn empty_carries_list_panics() {
    footman().with_resource_carrier(Vec::<(String, HarvestData)>::new());
}

#[test]
#[should_panic(expected = "carried resource kinds must not be empty")]
fn empty_carry_kind_panics() {
    footman().with_resource_carrier([(
        "",
        HarvestData::new(5, 5, 2, WorkPresence::Present, Banking::Carried),
    )]);
}

#[test]
#[should_panic(expected = "accepts must not be empty")]
fn empty_storage_accepts_panics() {
    footman().with_resource_storage(Vec::<String>::new());
}

#[test]
#[should_panic(expected = "accepted resource kinds must not be empty")]
fn empty_storage_kind_panics() {
    footman().with_resource_storage(["gold", ""]);
}

//
// ─── Berths ───────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a berth group must have at least one point")]
fn berth_group_without_points_panics() {
    BerthGroup::new(Vec::<FixedUVec2>::new(), 1);
}

#[test]
#[should_panic(expected = "a berth group must seat at least one worker")]
fn berth_group_without_slots_panics() {
    BerthGroup::new([middle()], 0);
}

#[test]
#[should_panic(expected = "a berth group cannot seat more workers than it has points")]
fn berth_group_seating_more_than_its_points_panics() {
    BerthGroup::new([middle()], 2);
}

#[test]
#[should_panic(expected = "berth groups must not be empty")]
fn berths_without_groups_panics() {
    BerthsDef::new(Vec::<(String, BerthGroup)>::new());
}

#[test]
#[should_panic(expected = "berth group names must not be empty")]
fn empty_berth_group_name_panics() {
    BerthsDef::new([("", BerthGroup::new([middle()], 1))]);
}

#[test]
#[should_panic(expected = "berths must not be empty")]
fn attachment_without_group_panics() {
    Attachment::new("", BerthStance::Still);
}

#[test]
#[should_panic(expected = "a berth-to-berth speed must be greater than 0")]
fn attachment_moving_at_no_speed_panics() {
    Attachment::new(
        "rim",
        BerthStance::Roaming {
            speed: FixedU64::ZERO,
            dwell: 4,
        },
    );
}

#[test]
#[should_panic(expected = "an orbit radius must be greater than 0")]
fn attachment_orbiting_at_no_distance_panics() {
    Attachment::new(
        "rim",
        BerthStance::Orbit {
            radius: FixedU64::ZERO,
            period: 4,
        },
    );
}

#[test]
#[should_panic(expected = "an orbit period must be greater than 0")]
fn attachment_orbiting_in_no_time_panics() {
    Attachment::new(
        "rim",
        BerthStance::Orbit {
            radius: FixedU64::ONE,
            period: 0,
        },
    );
}

//
// ─── Bonus damage ─────────────────────────────────────────────────────────────
//

#[test]
fn bonus_against_sums_type_and_tag_matches() {
    let def = footman().with_bonus_damage_vs([("keep", 10u32), ("building", 5u32)]);

    assert_eq!(def.bonus_against("keep", |tag| tag == "building"), 15);
}

#[test]
fn bonus_against_without_match_is_zero() {
    let def = footman().with_bonus_damage_vs([("building", 5u32)]);

    assert_eq!(def.bonus_against("footman", |_| false), 0);
}

#[test]
fn bonus_against_ignores_tags_absent_from_bonus_keys() {
    let def = footman().with_bonus_damage_vs([("building", 5u32)]);

    assert_eq!(def.bonus_against("footman", |tag| tag == "armored"), 0);
}

#[test]
#[should_panic(expected = "bonus_damage_vs keys must not be empty")]
fn empty_bonus_key_panics() {
    footman().with_bonus_damage_vs([("", 5u32)]);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The middle of a one-cell footprint, as a berth point.
fn middle() -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
}

fn footman() -> EntityTypeDef {
    utils::standing("footman", GROUND)
}
