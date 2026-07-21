//! Content validation: every invalid [`EntityTypeDef`] must panic at
//! construction, not misbehave at runtime.

use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::LayerId, nav_size::NavSize};
use ferrets_simulation::{
    components::{
        dying::DyingStaticData,
        location::Solidity,
        resource::{DepletionPolicy, HarvestData, HarvestVisibility},
    },
    content::entity_type_def::EntityTypeDef,
};

//
// ─── Happy path ───────────────────────────────────────────────────────────────
//

#[test]
fn fully_loaded_definition_is_valid() {
    let def = EntityTypeDef::new("factotum")
        .with_location(GROUND, NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_num(0.5))
        .with_health(50)
        .with_dying(3, None)
        .with_attack(10, 1, 1, 2, 2)
        .with_cost([("gold", 30), ("wood", 10)])
        .with_train_time(4)
        .with_build_time(6)
        .with_trainer(["footman"])
        .with_builder(["depot"])
        .with_resource_source("gold", DepletionPolicy::Destroy)
        .with_resource_carrier([("gold", HarvestData::new(5, 2, HarvestVisibility::Hidden))])
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
    EntityTypeDef::new("footman").with_location(LayerMask::EMPTY, NavSize::ONE, Solidity::Solid);
}

#[test]
#[should_panic(expected = "size dimensions must be greater than 0")]
fn zero_footprint_panics() {
    EntityTypeDef::new("footman").with_location(GROUND, NavSize::new(0, 2), Solidity::Solid);
}

//
// ─── Movement, health, attack ─────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "speed must be greater than 0")]
fn zero_speed_panics() {
    footman().with_movement(FixedU64::ZERO);
}

#[test]
#[should_panic(expected = "max_health must be greater than 0")]
fn zero_max_health_panics() {
    footman().with_health(0);
}

#[test]
#[should_panic(expected = "dying_time must be greater than 0")]
fn zero_dying_time_panics() {
    DyingStaticData::new(0, None);
}

#[test]
#[should_panic(expected = "corpse_type must not be empty")]
fn empty_corpse_type_panics() {
    DyingStaticData::new(3, Some(""));
}

#[test]
#[should_panic(expected = "aiming must be greater than 0")]
fn zero_aiming_panics() {
    footman().with_attack(10, 1, 1, 0, 2);
}

#[test]
#[should_panic(expected = "reloading must be greater than 0")]
fn zero_reloading_panics() {
    footman().with_attack(10, 1, 1, 2, 0);
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
    footman().with_builder(Vec::<String>::new());
}

#[test]
#[should_panic(expected = "constructed type names must not be empty")]
fn empty_builds_entry_panics() {
    footman().with_builder([""]);
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
    HarvestData::new(5, 0, HarvestVisibility::Visible);
}

#[test]
#[should_panic(expected = "capacity must be greater than 0")]
fn zero_carry_capacity_panics() {
    HarvestData::new(0, 2, HarvestVisibility::Visible);
}

#[test]
#[should_panic(expected = "carries must not be empty")]
fn empty_carries_list_panics() {
    footman().with_resource_carrier(Vec::<(String, HarvestData)>::new());
}

#[test]
#[should_panic(expected = "carried resource kinds must not be empty")]
fn empty_carry_kind_panics() {
    footman().with_resource_carrier([("", HarvestData::new(5, 2, HarvestVisibility::Visible))]);
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
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

const GROUND: LayerId = LayerId::new(1);

fn footman() -> EntityTypeDef {
    EntityTypeDef::new("footman").with_location(GROUND, NavSize::ONE, Solidity::Solid)
}
