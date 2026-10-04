//! Per-tick stat pipeline: fold active buffs into effective stats, and move
//! each pool as the modifiers on its maximum come and go.

use bevy_ecs::{change_detection::Mut, entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    annex,
    components::{
        entity_buffs::BuffsComponent,
        entity_stats::StatsComponent,
        pool_shifts::{self, PoolShiftTerms, PoolShiftsComponent},
        pools,
    },
    entity_def,
    entity_index::EntityIndex,
    fields,
    player_buffs::PlayerBuffs,
    player_stats::PlayerStats,
    session::{GameSession, player_id::PlayerId},
};
use ferrets_content::{
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    pool_def::PoolId,
    registry::ContentRegistry,
    stats::{EntityModifier, PlayerModifier},
};

/// Recomputes every entity's effective stats — the once-per-tick snapshot the
/// rest of the tick reads — from its base stats and the entity modifiers that
/// reach it: its own buffs, its owner's buffs and applied modifiers, which
/// cover every unit the owner has, and the field effects that hold for where
/// it stands. Runs before the systems that consume
/// stats, so a buff applied by a command this tick is already in effect this
/// tick. The dying are out of the alive index and keep their last snapshot.
pub fn recompute_entity_stats(world: &mut World) {
    // Gather first — reads only — so the apply pass below can take the world
    // mutably. The owner-side lists are the same for every unit an owner has,
    // so they are gathered once per player, not once per entity.
    let owner_modifiers: Vec<Vec<EntityModifiers>> =
        (0..world.resource::<GameSession>().slots().len())
            .map(|player| owner_entity_modifiers(world, player as PlayerId))
            .collect();

    let mut folds: Vec<(Entity, Vec<EntityModifiers>, &[EntityModifiers])> = Vec::new();
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !world.entity(entity).contains::<StatsComponent>() {
            continue;
        }
        let owned = entity_def::owner(world, entity)
            .map_or(&[][..], |owner| owner_modifiers[owner as usize].as_slice());
        folds.push((entity, own_and_standing(world, entity), owned));
    }
    fold(world, folds);
}

/// Recomputes `entity`'s effective stats alone, from the same modifiers
/// [`recompute_entity_stats`] folds for it, moving its pools as that pass
/// does. Panics when `entity` has no stat store.
pub fn recompute_stats_of(world: &mut World, entity: Entity) {
    assert!(
        world.entity(entity).contains::<StatsComponent>(),
        "recompute_stats_of is given an entity with a stat store"
    );
    let owned = entity_def::owner(world, entity)
        .map_or_else(Vec::new, |owner| owner_entity_modifiers(world, owner));
    let own = own_and_standing(world, entity);
    fold(world, vec![(entity, own, &owned)]);
}

/// Recomputes every player's effective stats: the player's own buffs fold
/// together with the modifiers applied to the player directly. Runs beside
/// [`recompute_entity_stats`], so a buff's grant appears the tick it is
/// applied and leaves with it.
///
/// Modifiers descend, never climb: a buff sitting on an entity never reaches
/// its owner's player stats, so only the player's own buffs are read here.
pub fn recompute_player_stats(world: &mut World) {
    let player_count = world.resource::<GameSession>().slots().len();

    let registry = world.resource::<ContentRegistry>();
    let player_buffs = world.resource::<PlayerBuffs>();
    let derived: Vec<Vec<PlayerModifier>> = (0..player_count)
        .map(|player| player_buff_player_modifiers(registry, player_buffs, player as PlayerId))
        .collect();

    let mut player_stats = world.resource_mut::<PlayerStats>();
    for (player, grants) in derived.into_iter().enumerate() {
        player_stats.set_derived(player as PlayerId, grants);
    }
}

/// What an entity's active buffs lay over it, resolved through the registry:
/// the entity modifiers of each modifier effect of each buff, its modifiers
/// once for each stack.
fn entity_buff_modifiers(
    registry: &ContentRegistry,
    buffs: &BuffsComponent,
) -> Vec<EntityModifiers> {
    let mut entity_modifiers = Vec::new();
    for (id, stacks) in buffs.active() {
        let buff = registry.entity_buff_def(id);
        for effect in &buff.effects {
            match effect {
                EntityEffect::Modifiers(modifiers) => {
                    entity_modifiers.push(repeated(modifiers, stacks));
                }
                EntityEffect::Disable | EntityEffect::Conceal => {}
            }
        }
    }
    entity_modifiers
}

/// What a player's active buffs lay over every owned unit: each set of
/// entity modifiers of each buff, its modifiers once for each stack.
fn player_buff_entity_modifiers(
    registry: &ContentRegistry,
    buffs: &PlayerBuffs,
    player: PlayerId,
) -> Vec<EntityModifiers> {
    buffs
        .active(player)
        .flat_map(|(id, stacks)| {
            registry
                .player_buff_def(id)
                .entity_modifiers
                .iter()
                .map(move |modifiers| repeated(modifiers, stacks))
        })
        .collect()
}

/// The player modifiers a player's active buffs contribute to its own stats. A
/// buff with `n` stacks contributes its modifiers `n` times.
fn player_buff_player_modifiers(
    registry: &ContentRegistry,
    buffs: &PlayerBuffs,
    player: PlayerId,
) -> Vec<PlayerModifier> {
    let mut modifiers = Vec::new();
    for (id, stacks) in buffs.active(player) {
        let buff = registry.player_buff_def(id);
        for _ in 0..stacks {
            modifiers.extend_from_slice(&buff.player_modifiers);
        }
    }
    modifiers
}

/// What `player`'s buffs and applied modifiers lay over every unit it owns.
fn owner_entity_modifiers(world: &World, player: PlayerId) -> Vec<EntityModifiers> {
    let registry = world.resource::<ContentRegistry>();
    let mut entity_modifiers =
        player_buff_entity_modifiers(registry, world.resource::<PlayerBuffs>(), player);
    entity_modifiers.extend(
        world
            .resource::<PlayerStats>()
            .entity_modifiers(player)
            .iter()
            .cloned(),
    );
    entity_modifiers
}

/// What reaches `entity` of its own: its buffs', then the fields' and the
/// annex's that hold for where it stands.
fn own_and_standing(world: &World, entity: Entity) -> Vec<EntityModifiers> {
    let registry = world.resource::<ContentRegistry>();
    let mut entity_modifiers = match world.entity(entity).get::<BuffsComponent>() {
        Some(buffs) => entity_buff_modifiers(registry, buffs),
        None => Vec::new(),
    };
    entity_modifiers.extend(fields::entity_modifiers(world, entity));
    let alone = annex::modifiers(world, entity);
    entity_modifiers.extend((!alone.is_empty()).then_some(EntityModifiers::Stats(alone)));
    entity_modifiers
}

/// Folds each entity's own entity modifiers and its owner's (`owned`, shared
/// by every unit the owner has) into its effective stats, each stat held at
/// the floor its registration carries, and moves its pools as the parts of
/// their maxima came and went.
fn fold(world: &mut World, folds: Vec<(Entity, Vec<EntityModifiers>, &[EntityModifiers])>) {
    // The floors live in the registry alone, held aside for the pass so the
    // entities it folds can be reached at the same time.
    world.resource_scope(|world, registry: Mut<ContentRegistry>| {
        for (entity, own, owned) in folds {
            let modifiers: Vec<EntityModifier> = own
                .iter()
                .chain(owned)
                .flat_map(|modifiers| modifiers.modifiers().iter().copied())
                .collect();
            world
                .entity_mut(entity)
                .get_mut::<StatsComponent>()
                .expect("a folded entity carries a stat store")
                .recompute(&modifiers, registry.entity_stat_defs());
            follow_pools(world, &registry, entity, &own, owned);
        }
    });
}

/// Moves each of `entity`'s pools as the parts of its maximum came and went
/// since the last fold, and remembers the parts folded now. A pool the
/// entity does not have yet is left to whoever fills it.
fn follow_pools(
    world: &mut World,
    registry: &ContentRegistry,
    entity: Entity,
    own: &[EntityModifiers],
    owned: &[EntityModifiers],
) {
    let entity_ref = world.entity(entity);
    let stats = entity_ref
        .get::<StatsComponent>()
        .expect("a folded entity carries a stat store");
    let remembered = entity_ref
        .get::<PoolShiftsComponent>()
        .expect("a simulation entity carries its pool shifts");
    // Judged first, against the store as it stands; nothing is written for an
    // entity whose maxima the same parts reach as last time.
    let mut changes: Vec<(PoolId, Vec<PoolShiftTerms>, Option<FixedU64>)> = Vec::new();
    for pool in registry
        .def(entity_def::type_id(world, entity))
        .base_stats
        .pools()
        .map(|pool| pool.id())
    {
        let maximum_stat = registry.pool_def(pool).maximum_stat();
        let base = stats
            .base(maximum_stat)
            .expect("a pool the type declares seeds its maximum");
        let now = pool_shifts::parts_of(own.iter().chain(owned), maximum_stat);
        let before = remembered.parts(pool);
        if before == now.as_slice() {
            continue;
        }
        let floor = registry.entity_stat_def(maximum_stat).floor();
        let current = entity_def::pool_value(world, entity, pool)
            .map(|current| pool_shifts::stepped(pool, current, base, floor, before, &now));
        changes.push((pool, now, current));
    }
    for (pool, parts, current) in changes {
        if let Some(current) = current {
            pools::follow(world, registry, entity, pool, current);
        }
        world
            .entity_mut(entity)
            .get_mut::<PoolShiftsComponent>()
            .expect("a simulation entity carries its pool shifts")
            .remember(pool, parts);
    }
}

/// `modifiers` once for each of `stacks`.
fn repeated(modifiers: &EntityModifiers, stacks: u32) -> EntityModifiers {
    let repeated = |modifiers: &[EntityModifier]| {
        (0..stacks)
            .flat_map(|_| modifiers.iter().copied())
            .collect()
    };
    match modifiers {
        EntityModifiers::Stats(modifiers) => EntityModifiers::Stats(repeated(modifiers)),
        EntityModifiers::PoolMaximums {
            modifiers,
            pool_shift,
        } => EntityModifiers::PoolMaximums {
            modifiers: repeated(modifiers),
            pool_shift: *pool_shift,
        },
    }
}
