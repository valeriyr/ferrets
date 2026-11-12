//! Spawn contracts: what an entity may arrive as, and what a placement may start.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    components::{
        field_source::FieldReach,
        rise::{RiseAttendance, RiseOrigin},
    },
    entity_def,
    simulation_id::SimulationId,
    spawn::{self, Arrival, At},
};

mod utils;

//
// ─── Arrival ──────────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a rising entity is of a type that rises: 'unit'")]
fn rising_arrival_of_type_that_never_rises_panics() {
    let mut world = utils::world([Pool::health(100)]);
    spawn::create_entity(
        &mut world,
        "unit",
        at_cell(5, 5),
        None,
        FieldReach::Initial,
        Arrival::Rising {
            attendance: RiseAttendance::Halted,
            origin: RiseOrigin::Founded { payer: None },
        },
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

/// Spawning anchored at the cell `(x, y)`.
fn at_cell(x: u32, y: u32) -> At {
    At::Cell(CellPos::new(x, y))
}

/// Places an owned `unit` at cell `(5, 5)` with `starts`.
fn place_unit(
    world: &mut World,
    starts: &[(PoolId, PoolInitial)],
) -> Option<(Entity, SimulationId)> {
    spawn::spawn_placed(
        world,
        "unit",
        at_cell(5, 5),
        Some(0),
        FieldReach::Full,
        starts,
    )
}
