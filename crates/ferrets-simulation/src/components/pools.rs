//! The current value of each pool an entity has, and what it follows.

use bevy_ecs::prelude::*;
use ferrets_content::{pool::PoolInitial, pool_def::PoolId, registry::ContentRegistry};
use ferrets_math::FixedU64;

use crate::entity_def;

/// The current value of each pool an entity has, and what it follows; every
/// simulation entity carries one, empty when it has no pool.
#[derive(Component, Debug, Clone, Default)]
pub struct PoolsComponent(Vec<PoolEntry>);

/// One pool an entity has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolEntry {
    /// The pool.
    pool: PoolId,
    /// Its current value.
    value: FixedU64,
    /// What its value follows.
    follows: Follows,
}

/// What a pool's value follows when its maximum or a site's work moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follows {
    /// Its maximum, by the shift of each part that comes or goes.
    Maximum,
    /// A construction site's work, up to the fold after the site is built,
    /// which settles the pool from the line it last stood at.
    Work {
        /// Where the work's line stood when last read.
        line: FixedU64,
    },
}

impl PoolEntry {
    /// The pool.
    #[inline]
    pub fn pool(&self) -> PoolId {
        self.pool
    }

    /// Its current value.
    #[inline]
    pub fn value(&self) -> FixedU64 {
        self.value
    }

    /// What its value follows.
    #[inline]
    pub fn follows(&self) -> Follows {
        self.follows
    }
}

impl PoolsComponent {
    /// The current value of `pool`, if the entity has it.
    pub fn current(&self, pool: PoolId) -> Option<FixedU64> {
        self.entry(pool).map(PoolEntry::value)
    }

    /// What `pool`'s value follows, if the entity has it.
    pub fn follows(&self, pool: PoolId) -> Option<Follows> {
        self.entry(pool).map(PoolEntry::follows)
    }

    /// Whether some pool of the entity follows a site's work.
    pub fn follows_work(&self) -> bool {
        self.0.iter().any(|entry| match entry.follows {
            Follows::Work { .. } => true,
            Follows::Maximum => false,
        })
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

    /// Each pool the entity has.
    pub fn iter(&self) -> impl Iterator<Item = &PoolEntry> + '_ {
        self.0.iter()
    }

    /// Whether the entity has no pool at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The entry of `pool`, if the entity has it.
    fn entry(&self, pool: PoolId) -> Option<&PoolEntry> {
        self.0.iter().find(|entry| entry.pool == pool)
    }

    /// Gives the entity `pool` at `value`, following `follows`. Panics when
    /// the entity already has `pool`.
    fn insert(&mut self, pool: PoolId, value: FixedU64, follows: Follows) {
        assert!(
            self.entry(pool).is_none(),
            "a pool inserted is one the entity lacks: {pool:?}"
        );
        self.0.push(PoolEntry {
            pool,
            value,
            follows,
        });
    }

    /// Sets `pool` to `value`. Panics when the entity does not have `pool`.
    fn set(&mut self, pool: PoolId, value: FixedU64) {
        self.held(pool).value = value;
    }

    /// Sets what `pool`'s value follows. Panics when the entity does not have
    /// `pool`.
    fn follow(&mut self, pool: PoolId, follows: Follows) {
        self.held(pool).follows = follows;
    }

    /// Takes `pool` off the entity.
    fn remove(&mut self, pool: PoolId) {
        self.0.retain(|entry| entry.pool != pool);
    }

    /// Lowers `pool` by `amount`, to no lower than empty. Panics when the
    /// entity does not have `pool`.
    fn drain(&mut self, pool: PoolId, amount: FixedU64) {
        let entry = self.held(pool);
        entry.value = entry.value.saturating_sub(amount);
    }

    /// Takes `cost` out of `pool` if it holds that much. Panics when the
    /// entity does not have `pool`.
    fn spend(&mut self, pool: PoolId, cost: FixedU64) -> Spending {
        let entry = self.held(pool);
        if entry.value >= cost {
            entry.value -= cost;
            Spending::Paid
        } else {
            Spending::Short
        }
    }

    /// The entry of `pool`, to change in place. Panics when the entity does
    /// not have `pool`.
    fn held(&mut self, pool: PoolId) -> &mut PoolEntry {
        self.0
            .iter_mut()
            .find(|entry| entry.pool == pool)
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

/// Gives `entity` the `pool`, started at `initial` against its effective
/// maximum and following that maximum. Panics when the entity already has
/// the pool or carries no maximum for it.
pub fn seed(world: &mut World, entity: Entity, pool: PoolId, initial: PoolInitial) {
    let value = start(world, entity, pool, initial);
    pools_of(world, entity).insert(pool, value, Follows::Maximum);
}

/// Gives `entity` the `pool`, started at `initial` against its effective
/// maximum and following a site's work from there, its line standing at that
/// start. Panics when the entity already has the pool or carries no maximum
/// for it.
pub fn seed_rising(world: &mut World, entity: Entity, pool: PoolId, initial: PoolInitial) {
    let line = start(world, entity, pool, initial);
    pools_of(world, entity).insert(pool, line, Follows::Work { line });
}

/// Starts `entity`'s `pool`, which follows its maximum, at `initial` against
/// that maximum again. Panics when the pool does not follow its maximum.
pub fn reseed(world: &mut World, entity: Entity, pool: PoolId, initial: PoolInitial) {
    match follows(world, entity, pool) {
        Some(Follows::Maximum) => {}
        Some(Follows::Work { .. }) | None => {
            panic!("reseed is given a pool following its maximum: {pool:?}")
        }
    }
    let value = start(world, entity, pool, initial);
    pools_of(world, entity).set(pool, value);
}

/// Starts `entity`'s `pool`, which follows a site's work, at `initial`
/// against its effective maximum again, its line with it. Panics when the
/// pool does not follow work.
pub fn reseed_rising(world: &mut World, entity: Entity, pool: PoolId, initial: PoolInitial) {
    match follows(world, entity, pool) {
        Some(Follows::Work { .. }) => {}
        Some(Follows::Maximum) | None => {
            panic!("reseed_rising is given a pool following work: {pool:?}")
        }
    }
    let line = start(world, entity, pool, initial);
    let mut pools = pools_of(world, entity);
    pools.set(pool, line);
    pools.follow(pool, Follows::Work { line });
}

/// Sets `entity`'s `pool`, which follows a site's work, to `value`, its line
/// now standing at `line`. Panics when the pool does not follow work, or
/// `value` is above its effective maximum.
pub fn step_with_work(
    world: &mut World,
    registry: &ContentRegistry,
    entity: Entity,
    pool: PoolId,
    value: FixedU64,
    line: FixedU64,
) {
    match follows(world, entity, pool) {
        Some(Follows::Work { .. }) => {}
        Some(Follows::Maximum) | None => {
            panic!("step_with_work is given a pool following work: {pool:?}")
        }
    }
    let maximum = maximum(world, registry, entity, pool);
    assert!(
        value <= maximum,
        "a pool holds no more than its maximum: {pool:?} {value} > {maximum}"
    );
    let mut pools = pools_of(world, entity);
    pools.set(pool, value);
    pools.follow(pool, Follows::Work { line });
}

/// Puts `entity`'s `pool`, which follows a site's work, back to following
/// its maximum. Panics when the pool does not follow work.
pub fn follow_maximum(world: &mut World, entity: Entity, pool: PoolId) {
    let mut pools = pools_of(world, entity);
    match pools.follows(pool) {
        Some(Follows::Work { .. }) => pools.follow(pool, Follows::Maximum),
        Some(Follows::Maximum) | None => {
            panic!("follow_maximum is given a pool following work: {pool:?}")
        }
    }
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

/// Sets `entity`'s `pool` to `value`. Panics when the entity does not have
/// the pool, or `value` is above its effective maximum.
pub fn follow_pool(
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
    pools_of(world, entity).map_unchanged(|pools| &mut pools.held(pool).value)
}

/// What `initial` comes to under `entity`'s effective maximum of `pool`.
/// Panics when the entity carries no maximum for the pool.
fn start(world: &World, entity: Entity, pool: PoolId, initial: PoolInitial) -> FixedU64 {
    initial.under(maximum(
        world,
        world.resource::<ContentRegistry>(),
        entity,
        pool,
    ))
}

/// What `entity`'s `pool` follows, if the entity has it.
fn follows(world: &World, entity: Entity, pool: PoolId) -> Option<Follows> {
    world
        .get::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store")
        .follows(pool)
}
