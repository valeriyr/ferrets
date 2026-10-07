//! Spawn contracts: what an entity may arrive as, and what a placement may start.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    components::build::SiteWork,
    entity_def,
    simulation_id::SimulationId,
    spawn::{self, Arrival, FieldReach},
};

mod utils;

//
// ─── Arrival ──────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a site is of a type that can be built: 'unit'")]
fn site_of_type_that_cannot_be_built_panics() {
    let mut world = utils::world([Pool::health(100)]);
    spawn::create_entity(
        &mut world,
        "unit",
        cell(5, 5),
        None,
        FieldReach::Initial,
        Arrival::Site(SiteWork::Halted),
    );
}

//
// ─── Placement starts ─────────────────────────────────────────────────────────
//

#[test]
fn placement_starts_pool_where_it_names() {
    let mut world = utils::world([Pool::health(100)]);
    let (unit, _) = place_unit(
        &mut world,
        &[(PoolId::HEALTH, PoolInitial::Share(utils::fixed("0.5")))],
    )
    .expect("the cell takes a unit");
    // Half of 100.
    assert_eq!(
        entity_def::pool_value(&world, unit, PoolId::HEALTH),
        Some(FixedU64::from_num(50))
    );
}

#[test]
#[should_panic(expected = "'unit' starts the energy pool, which its type does not declare")]
fn placement_starting_undeclared_pool_panics() {
    let mut world = utils::world([Pool::health(100)]);
    place_unit(&mut world, &[(PoolId::ENERGY, PoolInitial::Full)]);
}

#[test]
#[should_panic(expected = "'unit' starts the health pool twice")]
fn placement_starting_pool_twice_panics() {
    let mut world = utils::world([Pool::health(100)]);
    place_unit(
        &mut world,
        &[
            (PoolId::HEALTH, PoolInitial::Full),
            (PoolId::HEALTH, PoolInitial::Full),
        ],
    );
}

#[test]
#[should_panic(expected = "'unit' starts the health pool at 0")]
fn placement_starting_health_empty_panics() {
    let mut world = utils::world([Pool::health(100)]);
    place_unit(
        &mut world,
        &[(PoolId::HEALTH, PoolInitial::Amount(FixedU64::ZERO))],
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The origin corner of cell `(x, y)`.
fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// Places an owned `unit` at cell `(5, 5)` with `starts`.
fn place_unit(
    world: &mut World,
    starts: &[(PoolId, PoolInitial)],
) -> Option<(Entity, SimulationId)> {
    spawn::spawn_placed(world, "unit", cell(5, 5), Some(0), FieldReach::Full, starts)
}
