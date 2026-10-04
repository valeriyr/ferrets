//! The per-entity pool store, the integer readings of health and energy, and the writes that change a pool.

use bevy_ecs::{change_detection::Mut, entity::Entity, world::World};
use ferrets_content::{pool::Pool, pool_def::PoolId, registry::ContentRegistry};
use ferrets_math::FixedU64;
use ferrets_simulation::components::pools::{self, PoolsComponent, Spending};

mod utils;

//
// ─── Store ────────────────────────────────────────────────────────────────────
//

#[test]
fn emptied_needs_pool_held_at_zero() {
    // 1 − 0.5 left is not emptied; 1 − 1 = 0 is; a pool the unit lacks is not.
    let (mut world, unit) = unit_with([Pool::health(1)]);
    pools::drain(&mut world, unit, PoolId::HEALTH, fixed("0.5"));
    assert!(!store_of(&world, unit).emptied(PoolId::HEALTH));
    pools::drain(&mut world, unit, PoolId::HEALTH, fixed("0.5"));
    assert!(store_of(&world, unit).emptied(PoolId::HEALTH));
    assert!(!store_of(&world, unit).emptied(PoolId::ENERGY));
}

#[test]
fn store_is_empty_only_without_any_pool() {
    assert!(PoolsComponent::default().is_empty());
    let (mut world, unit) = unit_with([Pool::health(1)]);
    pools::drain(&mut world, unit, PoolId::HEALTH, fixed("1"));
    assert!(
        !store_of(&world, unit).is_empty(),
        "an emptied pool is still a pool"
    );
}

//
// ─── Displayed points ─────────────────────────────────────────────────────────
//

#[test]
fn full_health_displays_maximum() {
    assert_eq!(pools::displayed_health(FixedU64::from_num(50)), 50);
}

#[test]
fn sub_point_damage_stays_visible() {
    // 50 − 0.5 = 49.5: a fraction of a point lost still reads as lost (floor),
    // never rounded back up to full.
    assert_eq!(pools::displayed_health(fixed("49.5")), 49);
}

#[test]
fn barely_alive_health_never_displays_zero() {
    // 0.5 is below a whole point but still alive, so it reads as 1, not 0.
    assert_eq!(pools::displayed_health(fixed("0.5")), 1);
}

#[test]
fn emptied_health_displays_zero() {
    assert_eq!(pools::displayed_health(FixedU64::ZERO), 0);
}

#[test]
fn energy_displays_whole_points_held() {
    // 20.75 shows 20, and 0.5 shows 0: a fraction short of a point is not one.
    assert_eq!(pools::displayed_energy(fixed("20.75")), 20);
    assert_eq!(pools::displayed_energy(fixed("0.5")), 0);
}

//
// ─── Writes ───────────────────────────────────────────────────────────────────
//

#[test]
fn spawned_pool_starts_at_its_maximum() {
    let (world, unit) = unit_with([Pool::energy(40)]);
    assert_eq!(current_as_u32(&world, unit, PoolId::ENERGY), 40);
}

#[test]
fn fill_tops_up_drained_pool_to_its_maximum() {
    // 40 − 25 = 15, filled back to the 40 maximum.
    let (mut world, unit) = unit_with([Pool::energy(40)]);
    pools::drain(&mut world, unit, PoolId::ENERGY, fixed("25"));
    pools::fill(&mut world, unit, PoolId::ENERGY);
    assert_eq!(current_as_u32(&world, unit, PoolId::ENERGY), 40);
}

#[test]
fn drain_stops_at_empty() {
    // 30 − 45 is held at 0.
    let (mut world, unit) = unit_with([Pool::health(30)]);
    pools::drain(&mut world, unit, PoolId::HEALTH, fixed("45"));
    assert_eq!(current_as_u32(&world, unit, PoolId::HEALTH), 0);
}

#[test]
#[should_panic(expected = "a pool changed is one the entity has")]
fn draining_pool_entity_lacks_panics() {
    let (mut world, unit) = unit_with([Pool::health(50)]);
    pools::drain(&mut world, unit, PoolId::ENERGY, fixed("1"));
}

#[test]
fn spend_takes_only_what_pool_holds() {
    // 30 covers 20, leaving 10; 10 does not cover 20, and stays.
    let (mut world, unit) = unit_with([Pool::energy(30)]);
    assert_eq!(
        pools::spend(&mut world, unit, PoolId::ENERGY, fixed("20")),
        Spending::Paid
    );
    assert_eq!(
        pools::spend(&mut world, unit, PoolId::ENERGY, fixed("20")),
        Spending::Short
    );
    assert_eq!(current_as_u32(&world, unit, PoolId::ENERGY), 10);
}

#[test]
fn removed_pool_is_gone_and_others_stay() {
    let (mut world, unit) = unit_with([Pool::health(50), Pool::energy(20)]);
    pools::remove(&mut world, unit, PoolId::HEALTH);
    assert!(!store_of(&world, unit).has(PoolId::HEALTH));
    assert_eq!(current_as_u32(&world, unit, PoolId::ENERGY), 20);
}

#[test]
#[should_panic(expected = "a pool changed is one the entity has")]
fn restoring_pool_entity_lacks_panics() {
    let (mut world, unit) = unit_with([Pool::health(50)]);
    pools::restore(&mut world, unit, PoolId::ENERGY, fixed("5"));
}

#[test]
#[should_panic(expected = "a pool holds no more than its maximum")]
fn following_past_maximum_panics() {
    let (mut world, unit) = unit_with([Pool::health(100)]);
    world.resource_scope(|world, registry: Mut<ContentRegistry>| {
        pools::follow(world, &registry, unit, PoolId::HEALTH, fixed("101"));
    });
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A world with one spawned unit carrying `pools`, each full.
fn unit_with(pools: impl IntoIterator<Item = Pool>) -> (World, Entity) {
    let mut world = utils::world(pools);
    let unit = utils::spawn_unit(&mut world, 5, 5);
    (world, unit)
}

/// The pool store `entity` carries.
fn store_of(world: &World, entity: Entity) -> &PoolsComponent {
    world
        .get::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store")
}

/// What `entity`'s `pool` holds, as a whole number. Panics when the entity
/// has no such pool or its value is not whole.
fn current_as_u32(world: &World, entity: Entity, pool: PoolId) -> u32 {
    let value = store_of(world, entity)
        .current(pool)
        .expect("the entity carries the pool");
    assert!(
        value.frac() == FixedU64::ZERO,
        "the entity's {pool:?} {value} is a whole number"
    );
    value.to_num::<u32>()
}

/// A fixed-point value parsed from decimal digits.
fn fixed(text: &str) -> FixedU64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a value"))
}
