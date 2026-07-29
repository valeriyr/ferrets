//! Per-tick stat pipeline: fold active buffs into effective stats, age timed
//! buffs, and advance the per-tick counters and pools that read those stats.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use crate::components::{
    buffs::{BuffId, BuffsComponent},
    energy::EnergyComponent,
    skills::SkillsComponent,
    stats::{Modifier, StatId, StatsComponent},
};
use crate::content::registry::ContentRegistry;

/// Applies the buff `id` to `entity`, inserting a [`BuffsComponent`] if it has
/// none. No-op for an entity with no stat store to modify.
pub fn apply_buff(world: &mut World, entity: Entity, id: BuffId) {
    if !world.entity(entity).contains::<StatsComponent>() {
        return;
    }
    let def = world.resource::<ContentRegistry>().buff_def(id);
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

/// Recomputes every buffed entity's effective stats from its base stats and its
/// active buffs — the once-per-tick snapshot the rest of the tick reads. Runs
/// before the systems that consume stats, so a buff applied by a command this
/// tick is already in effect this tick.
pub fn recompute_stats(world: &mut World) {
    world.resource_scope::<ContentRegistry, _>(|world, registry| {
        let mut query = world.query::<(&mut StatsComponent, &BuffsComponent)>();
        for (mut stats, buffs) in query.iter_mut(world) {
            stats.recompute(&buff_modifiers(&registry, buffs));
        }
    });
}

/// Ages timed buffs by one tick, dropping any that expire. Expiries take effect
/// at the next tick's [`recompute_stats`] snapshot.
pub fn process_buffs(world: &mut World) {
    let mut query = world.query::<&mut BuffsComponent>();
    for mut buffs in query.iter_mut(world) {
        buffs.tick_down();
    }
}

/// Ages every skill cooldown by one tick.
pub fn process_cooldowns(world: &mut World) {
    let mut query = world.query::<&mut SkillsComponent>();
    for mut skills in query.iter_mut(world) {
        skills.tick_cooldowns();
    }
}

/// Refills each energy pool by one tick's `energy_regen`, up to `max_energy`.
///
/// Runs unconditionally, so a pool also settles back under a ceiling a debuff has
/// lowered.
pub fn process_energy_regen(world: &mut World) {
    let mut query = world.query::<(&mut EnergyComponent, &StatsComponent)>();
    for (mut energy, stats) in query.iter_mut(world) {
        let max = stats
            .effective(StatId::MAX_ENERGY)
            .unwrap_or(FixedU64::ZERO);
        let regen = stats
            .effective(StatId::ENERGY_REGEN)
            .unwrap_or(FixedU64::ZERO);
        energy.regenerate(regen, max);
    }
}

/// The modifiers every active buff contributes, resolved through the registry. A
/// buff with `n` stacks contributes its modifiers `n` times.
fn buff_modifiers(registry: &ContentRegistry, buffs: &BuffsComponent) -> Vec<Modifier> {
    let mut modifiers = Vec::new();
    for (id, stacks) in buffs.active() {
        let buff = registry.buff_def(id);
        for _ in 0..stacks {
            modifiers.extend_from_slice(&buff.modifiers);
        }
    }
    modifiers
}
