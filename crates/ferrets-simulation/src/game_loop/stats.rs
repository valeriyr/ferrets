//! Per-tick stat pipeline: fold active buffs into effective stats, age timed
//! buffs, and advance the per-tick counters and pools that read those stats.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    components::{
        build::UnderConstructionComponent, energy::EnergyComponent, entity_buffs::BuffsComponent,
        entity_skills::SkillsComponent, entity_stats::StatsComponent, health::HealthComponent,
        owner::OwnerComponent,
    },
    content::{
        entity_buffs::EntityBuffId,
        entity_stats::EntityStatId,
        player_buffs::PlayerBuffId,
        registry::ContentRegistry,
        stats::{EntityModifier, PlayerModifier},
    },
    entity_index::EntityIndex,
    player_buffs::PlayerBuffs,
    player_skills::PlayerSkills,
    player_stats::PlayerStats,
    session::{GameSession, player_slot::PlayerId},
};

/// Applies the buff `id` to `entity`, inserting a [`BuffsComponent`] if it has
/// none. No-op for an entity with no stat store to modify.
pub fn apply_entity_buff(world: &mut World, entity: Entity, id: EntityBuffId) {
    if !world.entity(entity).contains::<StatsComponent>() {
        return;
    }
    let def = world.resource::<ContentRegistry>().entity_buff_def(id);
    let (stack_rule, duration) = (def.stack_rule, def.duration);
    let mut entity_mut = world.entity_mut(entity);
    if let Some(mut buffs) = entity_mut.get_mut::<BuffsComponent>() {
        buffs.apply(id, stack_rule, duration);
    } else {
        let mut buffs = BuffsComponent::default();
        buffs.apply(id, stack_rule, duration);
        entity_mut.insert(buffs);
    }
}

/// Recomputes every entity's effective stats — the once-per-tick snapshot the
/// rest of the tick reads — from its base stats and the entity modifiers that
/// reach it: its own buffs, plus its owner's buffs and applied modifiers,
/// which cover every unit the owner has. Runs before the systems that consume
/// stats, so a buff applied by a command this tick is already in effect this
/// tick. The dying are out of the alive index and keep their last snapshot.
pub fn recompute_entity_stats(world: &mut World) {
    // Gather first — reads only — so the apply pass below can take the world
    // mutably. The owner-side lists are the same for every unit an owner has,
    // so they are folded once per player, not once per entity.
    let registry = world.resource::<ContentRegistry>();
    let player_stats = world.resource::<PlayerStats>();
    let player_buffs = world.resource::<PlayerBuffs>();
    let owner_modifiers: Vec<Vec<EntityModifier>> =
        (0..world.resource::<GameSession>().slots().len())
            .map(|player| {
                let player = player as PlayerId;
                let mut modifiers = player_buff_entity_modifiers(registry, player_buffs, player);
                modifiers.extend_from_slice(player_stats.entity_modifiers(player));
                modifiers
            })
            .collect();

    let mut folds: Vec<(Entity, Vec<EntityModifier>)> = Vec::new();
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if !entity_ref.contains::<StatsComponent>() {
            continue;
        }
        let mut modifiers = match entity_ref.get::<BuffsComponent>() {
            Some(buffs) => entity_buff_modifiers(registry, buffs),
            None => Vec::new(),
        };
        if let Some(owner) = entity_ref.get::<OwnerComponent>() {
            modifiers.extend_from_slice(&owner_modifiers[owner.player() as usize]);
        }
        folds.push((entity, modifiers));
    }

    for (entity, modifiers) in folds {
        if let Some(mut stats) = world.entity_mut(entity).get_mut::<StatsComponent>() {
            stats.recompute(&modifiers);
        }
    }
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

/// Ages every entity's timed buffs by one tick, dropping any that expire.
/// Expiries take effect at the next tick's recompute snapshots.
pub fn process_entity_buffs(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if let Some(mut buffs) = world.entity_mut(entity).get_mut::<BuffsComponent>() {
            buffs.tick_down();
        }
    }
}

/// Ages every player's timed buffs by one tick, dropping any that expire.
/// Expiries take effect at the next tick's recompute snapshots.
pub fn process_player_buffs(world: &mut World) {
    world.resource_mut::<PlayerBuffs>().tick_down();
}

/// Applies the player-level buff `id` to `player`. The buff's own stacking rule
/// resolves a re-application, exactly as on an entity.
pub fn apply_player_buff(world: &mut World, player: PlayerId, id: PlayerBuffId) {
    let def = world.resource::<ContentRegistry>().player_buff_def(id);
    let (stack_rule, duration) = (def.stack_rule, def.duration);
    world
        .resource_mut::<PlayerBuffs>()
        .apply(player, id, stack_rule, duration);
}

/// Ages player-skill cooldowns by one tick. The buffs a cast applied age with
/// every other player buff in [`process_player_buffs`].
pub fn process_player_skills(world: &mut World) {
    world.resource_mut::<PlayerSkills>().tick_cooldowns();
}

/// Ages every entity-skill cooldown by one tick.
pub fn process_entity_skills(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if let Some(mut skills) = world.entity_mut(entity).get_mut::<SkillsComponent>() {
            skills.tick_cooldowns();
        }
    }
}

/// Refills each energy pool by one tick's `energy_regen`, up to `max_energy`.
///
/// Runs over the alive index, so the dying are already excluded. A pool also
/// settles back under a ceiling a debuff has lowered.
pub fn process_energy_regen(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if !entity_ref.contains::<EnergyComponent>() {
            continue;
        }
        // An energy pool is only ever seeded from `max_energy`, so anything with one
        // carries the stat. The regeneration rate is genuinely optional: a pool that
        // never refills on its own is ordinary content.
        let stats = entity_ref
            .get::<StatsComponent>()
            .expect("an energy pool implies the store it was seeded into");
        let max = stats
            .effective(EntityStatId::MAX_ENERGY)
            .expect("an energy pool implies the stat it was seeded from");
        let regen = stats
            .effective(EntityStatId::ENERGY_REGEN)
            .unwrap_or(FixedU64::ZERO);
        if let Some(mut energy) = world.entity_mut(entity).get_mut::<EnergyComponent>() {
            energy.regenerate(regen, max);
        }
    }
}

/// Refills each health pool by one tick's `health_regen`, up to `max_health`.
///
/// Runs over the alive index, so the dying are already excluded; entities still
/// under construction are skipped too — neither should mend on its own. A pool
/// also settles back under a ceiling a debuff has lowered.
pub fn process_health_regen(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<UnderConstructionComponent>() {
            continue;
        }
        let Some(health) = entity_ref.get::<HealthComponent>() else {
            continue;
        };
        // Nothing brings an entity back, whatever its regeneration says.
        if health.is_dead() {
            continue;
        }
        // A health pool is only ever seeded from `max_health`, so anything with one
        // carries the stat — and standing a missing ceiling in as zero would settle
        // the pool to zero and read as a kill.
        let stats = entity_ref
            .get::<StatsComponent>()
            .expect("a health pool implies the store it was seeded into");
        let max = stats
            .effective(EntityStatId::MAX_HEALTH)
            .expect("a health pool implies the stat it was seeded from");
        let regen = stats
            .effective(EntityStatId::HEALTH_REGEN)
            .unwrap_or(FixedU64::ZERO);
        if let Some(mut health) = world.entity_mut(entity).get_mut::<HealthComponent>() {
            health.heal(regen, max);
        }
    }
}

/// The modifiers an entity's active buffs contribute, resolved through the
/// registry. A buff with `n` stacks contributes its modifiers `n` times.
fn entity_buff_modifiers(
    registry: &ContentRegistry,
    buffs: &BuffsComponent,
) -> Vec<EntityModifier> {
    let mut modifiers = Vec::new();
    for (id, stacks) in buffs.active() {
        let buff = registry.entity_buff_def(id);
        for _ in 0..stacks {
            modifiers.extend_from_slice(&buff.modifiers);
        }
    }
    modifiers
}

/// The entity modifiers a player's active buffs lay over every owned unit. A
/// buff with `n` stacks contributes its modifiers `n` times.
fn player_buff_entity_modifiers(
    registry: &ContentRegistry,
    buffs: &PlayerBuffs,
    player: PlayerId,
) -> Vec<EntityModifier> {
    let mut modifiers = Vec::new();
    for (id, stacks) in buffs.active(player) {
        let buff = registry.player_buff_def(id);
        for _ in 0..stacks {
            modifiers.extend_from_slice(&buff.entity_modifiers);
        }
    }
    modifiers
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
