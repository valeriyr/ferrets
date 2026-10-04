//! The current value of each pool an entity has.

use bevy_ecs::prelude::*;
use ferrets_content::{pool_def::PoolId, registry::ContentRegistry};
use ferrets_math::FixedU64;

use crate::entity_def;

/// The current value of each pool an entity has; every simulation entity
/// carries one, empty when it has no pool.
#[derive(Component, Debug, Clone, Default)]
pub struct PoolsComponent(Vec<(PoolId, FixedU64)>);

impl PoolsComponent {
    /// The current value of `pool`, if the entity has it.
    pub fn current(&self, pool: PoolId) -> Option<FixedU64> {
        self.0
            .iter()
            .find(|(held, _)| *held == pool)
            .map(|&(_, value)| value)
    }

    /// Whether the entity has `pool`.
    pub fn has(&self, pool: PoolId) -> bool {
        self.current(pool).is_some()
    }

    /// Whether the entity has `pool` and it is empty.
    pub fn emptied(&self, pool: PoolId) -> bool {
        self.current(pool)
            .is_some_and(|value| value == FixedU64::ZERO)
    }

    /// Each pool the entity has, with its current value.
    pub fn iter(&self) -> impl Iterator<Item = (PoolId, FixedU64)> + '_ {
        self.0.iter().copied()
    }

    /// Whether the entity has no pool at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Sets `pool` to `value`, giving the entity the pool if it had none.
    fn set(&mut self, pool: PoolId, value: FixedU64) {
        match self.0.iter_mut().find(|(held, _)| *held == pool) {
            Some((_, held)) => *held = value,
            None => self.0.push((pool, value)),
        }
    }

    /// Takes `pool` off the entity.
    fn remove(&mut self, pool: PoolId) {
        self.0.retain(|&(held, _)| held != pool);
    }

    /// Lowers `pool` by `amount`, to no lower than empty. Panics when the
    /// entity does not have `pool`.
    fn drain(&mut self, pool: PoolId, amount: FixedU64) {
        let value = self.held(pool);
        *value = value.saturating_sub(amount);
    }

    /// Takes `cost` out of `pool` if it holds that much. Panics when the
    /// entity does not have `pool`.
    fn spend(&mut self, pool: PoolId, cost: FixedU64) -> Spending {
        let value = self.held(pool);
        if *value >= cost {
            *value -= cost;
            Spending::Paid
        } else {
            Spending::Short
        }
    }

    /// The value `pool` holds, to change in place. Panics when the entity does
    /// not have `pool`.
    fn held(&mut self, pool: PoolId) -> &mut FixedU64 {
        self.0
            .iter_mut()
            .find(|(held, _)| *held == pool)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("a pool changed is one the entity has: {pool:?}"))
    }
}

/// A health value as a whole number: `0` exactly when empty, otherwise at
/// least `1`.
pub fn displayed_health(value: FixedU64) -> u32 {
    if value == FixedU64::ZERO {
        0
    } else {
        value.to_num::<u32>().max(1)
    }
}

/// An energy value as a whole number: the whole points held, rounded down.
pub fn displayed_energy(value: FixedU64) -> u32 {
    value.to_num::<u32>()
}

/// Fills `entity`'s `pool` to its effective maximum, giving the entity the
/// pool if it had none. Panics when the entity carries no maximum for the
/// pool.
pub fn fill(world: &mut World, entity: Entity, pool: PoolId) {
    let value = maximum(world, world.resource::<ContentRegistry>(), entity, pool);
    pools_of(world, entity).set(pool, value);
}

/// Lowers `entity`'s `pool` by `amount`, to no lower than empty. Panics when
/// the entity does not have the pool.
pub fn drain(world: &mut World, entity: Entity, pool: PoolId, amount: FixedU64) {
    assert_has(world, entity, pool);
    pools_of(world, entity).drain(pool, amount);
}

/// What spending from a pool came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spending {
    /// The cost came out of the pool.
    Paid,
    /// The pool held less than the cost, and nothing came out.
    Short,
}

/// Takes `cost` out of `entity`'s `pool` if it holds that much. Panics when
/// the entity does not have the pool.
pub fn spend(world: &mut World, entity: Entity, pool: PoolId, cost: FixedU64) -> Spending {
    assert_has(world, entity, pool);
    pools_of(world, entity).spend(pool, cost)
}

/// Takes `pool` off `entity`. Panics when the entity does not have the pool.
pub fn remove(world: &mut World, entity: Entity, pool: PoolId) {
    assert_has(world, entity, pool);
    pools_of(world, entity).remove(pool);
}

/// Raises `entity`'s `pool` by `amount`, capped at its effective maximum.
/// Panics when the entity does not have the pool.
pub fn restore(world: &mut World, entity: Entity, pool: PoolId, amount: FixedU64) {
    assert_has(world, entity, pool);
    let maximum = maximum(world, world.resource::<ContentRegistry>(), entity, pool);
    let mut value = held(world, entity, pool);
    *value = value.saturating_add(amount).min(maximum);
}

/// Moves `entity`'s `pool` by one tick by `regen` net of `drain`, held
/// between empty and `maximum`, and answers what it holds after. Panics when
/// the entity does not have the pool.
pub fn flow(
    world: &mut World,
    entity: Entity,
    pool: PoolId,
    maximum: FixedU64,
    regen: FixedU64,
    drain: FixedU64,
) -> FixedU64 {
    let mut value = held(world, entity, pool);
    *value = value
        .saturating_add(regen)
        .saturating_sub(drain)
        .min(maximum);
    *value
}

/// Sets `entity`'s `pool` to `value`, the fold's write of a pool following
/// its maximum. Panics when the entity does not have the pool, or `value`
/// is above its effective maximum.
pub fn follow(
    world: &mut World,
    registry: &ContentRegistry,
    entity: Entity,
    pool: PoolId,
    value: FixedU64,
) {
    assert_has(world, entity, pool);
    let maximum = maximum(world, registry, entity, pool);
    assert!(
        value <= maximum,
        "a pool holds no more than its maximum: {pool:?} {value} > {maximum}"
    );
    *held(world, entity, pool) = value;
}

/// Panics when `entity` does not have `pool`.
fn assert_has(world: &World, entity: Entity, pool: PoolId) {
    assert!(
        entity_def::has_pool(world, entity, pool),
        "a pool changed is one the entity has: {pool:?}"
    );
}

/// The effective maximum of `entity`'s `pool`. Panics when the entity
/// carries no maximum for the pool.
fn maximum(world: &World, registry: &ContentRegistry, entity: Entity, pool: PoolId) -> FixedU64 {
    let maximum_stat = registry.pool_def(pool).maximum_stat();
    entity_def::effective_stat(world, entity, maximum_stat)
        .unwrap_or_else(|| panic!("a pool's entity carries its maximum: {pool:?}"))
}

/// `entity`'s pool store, to change in place. Panics when `entity` is no
/// simulation entity.
fn pools_of(world: &mut World, entity: Entity) -> Mut<'_, PoolsComponent> {
    world
        .get_mut::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store")
}

/// The value `entity`'s `pool` holds, to change in place. Panics when the
/// entity does not have the pool.
fn held(world: &mut World, entity: Entity, pool: PoolId) -> Mut<'_, FixedU64> {
    pools_of(world, entity).map_unchanged(|pools| pools.held(pool))
}
