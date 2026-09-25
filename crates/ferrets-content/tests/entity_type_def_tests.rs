//! Content validation: every invalid [`EntityTypeDef`] must panic at
//! construction, not misbehave at runtime.

mod utils;

use ferrets_content::{
    berths::{BerthGroup, BerthsDef},
    brood::{BreederDef, OrphanFate},
    build::BuilderAttendance,
    dying::{Bequest, DeathKind, DyingDef, LeftBy},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    kinds::Kinds,
    location::Solidity,
    quantity::Quantity,
    resource::{Banking, DepletionPolicy, HarvestData},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
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
        .with_dying(3, [])
        .with_attack(utils::weapon(GROUND), 10, 1, 1, 4, 2)
        .with_price([("gold", 30), ("wood", 10)])
        .with_train_time(4)
        .with_build_time(6)
        .with_trainer(["footman"])
        .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
        .with_builder(
            ["depot"],
            BuilderAttendance::Crew(WorkPresence::Hidden {
                crew: CrewLimit::ONE,
            }),
        )
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
#[should_panic(expected = "a dying time of no ticks is no dying time at all")]
fn dying_time_of_no_ticks_panics() {
    // None is how a type says it waits not at all; a stated zero is a number
    // meaning the same thing twice.
    DyingDef::new(Some(0), Vec::new());
}

#[test]
#[should_panic(expected = "entity_type must not be empty")]
fn empty_bequest_type_panics() {
    Bequest::new("", 1, LeftBy::Ordinary);
}

#[test]
#[should_panic(expected = "leaves none of it")]
fn bequest_of_none_panics() {
    Bequest::new("corpse", 0, LeftBy::Ordinary);
}

#[test]
#[should_panic(expected = "is left by no death at all")]
fn bequest_no_death_hands_on_panics() {
    Bequest::new("corpse", 1, LeftBy::Named(Vec::new()));
}

#[test]
#[should_panic(expected = "a bequest of 'corpse' names Killed twice")]
fn bequest_naming_one_death_twice_panics() {
    Bequest::new(
        "corpse",
        1,
        LeftBy::Named(vec![DeathKind::Killed, DeathKind::Killed]),
    );
}

#[test]
#[should_panic(expected = "a death leaves 'corpse' twice")]
fn leaving_one_type_twice_panics() {
    DyingDef::new(
        Some(2),
        vec![
            Bequest::new("corpse", 1, LeftBy::Ordinary),
            Bequest::new("corpse", 2, LeftBy::Ordinary),
        ],
    );
}

#[test]
fn ordinary_bequest_is_left_by_death_that_ends_life() {
    let bequest = Bequest::new("corpse", 1, LeftBy::Ordinary);

    assert!(bequest.left_by(DeathKind::Killed));
    assert!(bequest.left_by(DeathKind::Expired));
    // Taken off the board rather than killed: the site it founded swallowed it.
    assert!(!bequest.left_by(DeathKind::Consumed));
    assert!(!bequest.left_by(DeathKind::Canceled));
}

#[test]
fn named_bequest_is_left_by_exactly_what_it_names() {
    let bequest = Bequest::new("corpse", 1, LeftBy::Named(vec![DeathKind::Canceled]));

    assert!(bequest.left_by(DeathKind::Canceled));
    assert!(!bequest.left_by(DeathKind::Killed));
}

//
// ─── Cost and production ──────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "price resource kinds must not be empty")]
fn empty_price_kind_panics() {
    footman().with_price([("", 30)]);
}

#[test]
#[should_panic(expected = "price amounts must be greater than 0")]
fn zero_price_amount_panics() {
    footman().with_price([("gold", 0)]);
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
        BuilderAttendance::Crew(WorkPresence::Hidden {
            crew: CrewLimit::ONE,
        }),
    );
}

#[test]
#[should_panic(expected = "constructed type names must not be empty")]
fn empty_builds_entry_panics() {
    footman().with_builder(
        [""],
        BuilderAttendance::Crew(WorkPresence::Hidden {
            crew: CrewLimit::ONE,
        }),
    );
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
    HarvestData::new(
        5,
        5,
        0,
        WorkPresence::Present {
            crew: CrewLimit::ONE,
        },
        Banking::Carried,
        Kinds::Any,
    );
}

#[test]
#[should_panic(expected = "capacity must be greater than 0")]
fn zero_carry_capacity_panics() {
    HarvestData::new(
        0,
        0,
        2,
        WorkPresence::Present {
            crew: CrewLimit::ONE,
        },
        Banking::Carried,
        Kinds::Any,
    );
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
    BerthGroup::new(Vec::<FixedVec2>::new(), 1);
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
#[should_panic(expected = "breeds must not be empty")]
fn breeder_of_nothing_panics() {
    BreederDef::new("", Quantity::Constant(10), 2, 0, OrphanFate::Perish);
}

#[test]
#[should_panic(expected = "a brood limit must admit at least one broodling")]
fn breeder_with_limit_of_zero_panics() {
    BreederDef::new("grub", Quantity::Constant(10), 0, 0, OrphanFate::Perish);
}

#[test]
#[should_panic(expected = "a brood cannot owe more broodlings than its limit admits")]
fn breeder_owing_more_than_its_limit_panics() {
    BreederDef::new("grub", Quantity::Constant(10), 2, 3, OrphanFate::Perish);
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
fn middle() -> FixedVec2 {
    FixedVec2::new(FixedI64::from_num(0.5), FixedI64::from_num(0.5))
}

fn footman() -> EntityTypeDef {
    utils::standing("footman", GROUND)
}
