//! Per-tick stat pipeline: fold active buffs into effective stats, age timed
//! buffs, and advance the per-tick counters and pools that read those stats.

use bevy_ecs::{change_detection::Mut, entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::{
    annex,
    buffs_store::Term,
    components::{
        build::UnderConstructionComponent, energy::EnergyComponent, entity_buffs::BuffsComponent,
        entity_skills::SkillsComponent, entity_stats::StatsComponent, health::HealthComponent,
        lifetime::LifetimeComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SpendCause},
    fields,
    game_loop::cost,
    player_buffs::PlayerBuffs,
    player_skills::PlayerSkills,
    player_stats::PlayerStats,
    session::{GameSession, player_id::PlayerId},
    spawn,
};
use ferrets_content::{
    entity_buffs::{EntityBuffId, Interruption, Lasting},
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    player_buffs::PlayerBuffId,
    registry::ContentRegistry,
    stats::{EntityModifier, PlayerModifier},
};

/// Applies the buff `id` to `entity`, inserting a [`BuffsComponent`] if it has
/// none. No-op for an entity with no stat store to modify.
pub fn apply_entity_buff(world: &mut World, entity: Entity, id: EntityBuffId) {
    if !world.entity(entity).contains::<StatsComponent>() {
        return;
    }
    let def = world.resource::<ContentRegistry>().entity_buff_def(id);
    // The tick of application ages the term once before anything reads it,
    // so the seat is one above the term: a buff for `n` ticks stands through
    // the `n` ticks after the one it landed in, and an upkeep's first payment
    // falls a full period after it.
    let term = match &def.lasting {
        Lasting::Forever => Term::Forever,
        Lasting::For(ticks) => Term::For {
            remaining: *ticks + 1,
        },
        Lasting::Upkeep { period, .. } => Term::Upkeep {
            period: *period,
            due_in: *period + 1,
        },
    };
    let stack_rule = def.stack_rule;
    let mut entity_mut = world.entity_mut(entity);
    if let Some(mut buffs) = entity_mut.get_mut::<BuffsComponent>() {
        buffs.apply(id, stack_rule, term);
    } else {
        let mut buffs = BuffsComponent::default();
        buffs.apply(id, stack_rule, term);
        entity_mut.insert(buffs);
    }
}

/// Takes off `entity` every buff whose definition names `interruption` as
/// what cuts it short. An entity carrying no buffs at all carries none to cut.
pub fn interrupt_entity_buffs(world: &mut World, entity: Entity, interruption: Interruption) {
    let Some(buffs) = world.entity(entity).get::<BuffsComponent>() else {
        return;
    };
    let registry = world.resource::<ContentRegistry>();
    let interrupted: Vec<EntityBuffId> = buffs
        .active()
        .map(|(id, _)| id)
        .filter(|&id| {
            registry
                .entity_buff_def(id)
                .interrupted_by
                .contains(&interruption)
        })
        .collect();
    if interrupted.is_empty() {
        return;
    }
    if let Some(mut buffs) = world.entity_mut(entity).get_mut::<BuffsComponent>() {
        for id in interrupted {
            buffs.remove(id);
        }
    }
}

/// Takes every stack of `id` off `entity`. An entity that carries no buffs at
/// all carries none of this one.
pub fn remove_entity_buff(world: &mut World, entity: Entity, id: EntityBuffId) {
    if let Some(mut buffs) = world.entity_mut(entity).get_mut::<BuffsComponent>() {
        buffs.remove(id);
    }
}

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
        if let Some(owner) = entity_def::owner(world, entity) {
            modifiers.extend_from_slice(&owner_modifiers[owner as usize]);
        }
        modifiers.extend(fields::modifiers(world, entity));
        modifiers.extend(annex::modifiers(world, entity));
        folds.push((entity, modifiers));
    }

    // The fold holds every stat at the floor its registration carries, and the
    // registry is the only place that knows them — held aside for the pass so
    // the entities it folds can be reached at the same time.
    world.resource_scope(|world, registry: Mut<ContentRegistry>| {
        for (entity, modifiers) in folds {
            if let Some(mut stats) = world.entity_mut(entity).get_mut::<StatsComponent>() {
                stats.recompute(&modifiers, registry.entity_stat_defs());
            }
        }
    });
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

/// Ages every entity's buffs by one tick: timed ones that ran out are dropped,
/// and an upkeep whose payment falls due is paid from the bearer's pools and
/// its owner's stockpile — or dropped, the tick it cannot be. Expiries take
/// effect at the next tick's recompute snapshots.
///
/// Nobody pays for the unowned: an upkeep on an ownerless bearer ends at its
/// first due tick.
pub fn process_entity_buffs(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let due = match world.entity_mut(entity).get_mut::<BuffsComponent>() {
            Some(mut buffs) => buffs.tick_down(),
            None => continue,
        };
        for (id, stacks) in due {
            // Cloned off the registry borrow, which the payment needs released,
            // and taken once per stack: the modifiers are applied per stack, so
            // the upkeep is owed per stack too.
            let costs = match &world
                .resource::<ContentRegistry>()
                .entity_buff_def(id)
                .lasting
            {
                Lasting::Upkeep { costs, .. } => cost::times(costs, stacks),
                Lasting::Forever | Lasting::For(_) => {
                    unreachable!("the store reports a payment due on an upkeep alone")
                }
            };
            match entity_def::owner(world, entity) {
                Some(player) if cost::can_pay(world, entity, player, &costs) => {
                    let bearer = entity_def::simulation_id(world, entity);
                    cost::pay(
                        world,
                        entity,
                        player,
                        &costs,
                        SpendCause::Upkeep { bearer, buff: id },
                    );
                }
                // Unaffordable, or nobody's to pay for: the buff ends.
                Some(_) | None => remove_entity_buff(world, entity, id),
            }
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
    // The content still states a lifetime as an optional tick count, where
    // absence means forever; the store takes the term outright.
    let term = match def.duration {
        Some(ticks) => Term::For { remaining: ticks },
        None => Term::Forever,
    };
    let stack_rule = def.stack_rule;
    world
        .resource_mut::<PlayerBuffs>()
        .apply(player, id, stack_rule, term);
}

/// Takes the player-level buff `id` off `player`, however much of it was left.
pub fn remove_player_buff(world: &mut World, player: PlayerId, id: PlayerBuffId) {
    world.resource_mut::<PlayerBuffs>().remove(player, id);
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

/// Moves each health pool by one tick: up by `health_regen` to `max_health`,
/// then down by `health_drain`.
///
/// Runs over the alive index, so the dying are already excluded; entities still
/// under construction are skipped too. A pool also settles back under a ceiling
/// a debuff has lowered, and one a drain runs dry dies of it.
pub fn process_health_flow(world: &mut World) {
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
        let drain = stats
            .effective(EntityStatId::HEALTH_DRAIN)
            .unwrap_or(FixedU64::ZERO);
        let emptied = match world.entity_mut(entity).get_mut::<HealthComponent>() {
            Some(mut health) => {
                health.heal(regen, max);
                health.drain(drain);
                health.is_dead()
            }
            None => unreachable!("the pool read a moment ago is still the entity's own"),
        };
        // A pool this pass ran dry dies of it, with nobody to blame: a
        // structure withering off the field that sustains it. A pool that was
        // already empty is left where the opening rule left it.
        if emptied {
            spawn::despawn_entity(world, entity, DeathCause::Decayed);
        }
    }
}

/// Ages every timed life by one tick, ending the ones whose time is up.
///
/// Runs over the alive index, so the dying are already excluded. The age is
/// compared against the *effective* stat, so a buff that lengthens a life keeps
/// standing instances on their feet and one that shortens it takes them at
/// once.
pub fn process_lifetimes(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let Some(lifetime) = world.entity(entity).get::<LifetimeComponent>() else {
            continue;
        };
        // A timed life is only ever fitted from the stat, so anything carrying
        // one carries the stat.
        let limit = entity_def::effective_stat_u32(world, entity, EntityStatId::LIFETIME);
        let age = lifetime.age + 1;
        if age >= limit {
            spawn::despawn_entity(world, entity, DeathCause::Expired);
            continue;
        }
        world
            .entity_mut(entity)
            .get_mut::<LifetimeComponent>()
            .expect("the timed life read a moment ago is still the entity's own")
            .age = age;
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
        for effect in &buff.effects {
            match effect {
                EntityEffect::Modifiers(granted) => {
                    for _ in 0..stacks {
                        modifiers.extend_from_slice(granted);
                    }
                }
                EntityEffect::Disable | EntityEffect::Conceal => {}
            }
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
