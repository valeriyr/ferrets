//! The buffs entities and players bear: putting them on and taking them off,
//! how long they last, and which of them an entity holds while a requirement
//! is met or its form allows.

use bevy_ecs::{entity::Entity, world::World};

use crate::{
    buffs_store::{Held, Term},
    components::entity_buffs::BuffsComponent,
    entity_def,
    entity_index::EntityIndex,
    events::SpendCause,
    game_loop::cost,
    player_buffs::PlayerBuffs,
    requirements,
    session::player_id::PlayerId,
};
use ferrets_content::{
    entity_buffs::{EntityBuffId, Interruption, Lasting},
    entity_effect::EntityEffect,
    entity_type_def::EntityTypeId,
    player_buffs::PlayerBuffId,
    registry::ContentRegistry,
};

/// Applies the buff `id` to `entity`, inserting a [`BuffsComponent`] if it has
/// none, when the entity can carry it — see [`entity_def::can_carry`].
pub fn apply_entity_buff(world: &mut World, entity: Entity, id: EntityBuffId) -> Bearing {
    bear(world, entity, id, Held::Applied)
}

/// Whether a buff applied landed on its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bearing {
    /// The target bears the buff.
    Borne,
    /// The target lacks a stat the buff modifies, and bears nothing of it.
    Uncarried,
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
    for id in interrupted {
        remove_entity_buff(world, entity, id);
    }
}

/// Takes every stack of `id` off `entity`. Panics when `entity` does not bear
/// it.
pub fn remove_entity_buff(world: &mut World, entity: Entity, id: EntityBuffId) {
    assert!(
        entity_def::bears(world, entity, id),
        "remove_entity_buff is given a buff its entity bears: {id:?}"
    );
    world
        .entity_mut(entity)
        .get_mut::<BuffsComponent>()
        .expect("an entity bearing a buff carries a buff store")
        .remove(id);
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
                Lasting::Forever | Lasting::For(_) | Lasting::While(_) => {
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

/// Refits, on every alive entity, the buffs held on a requirement — see
/// [`refit_entity`].
pub fn refit_held_buffs(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        refit_entity(world, entity);
    }
}

/// Drops the passives the type `from` fitted on `entity` that the type it has
/// just changed into does not name. A buff applied by anything else keeps
/// holding on its own requirement.
pub fn shed_form(world: &mut World, entity: Entity, from: EntityTypeId) {
    let registry = world.resource::<ContentRegistry>();
    let now = &entity_def::of(world, entity).passives;
    let buffs = world.entity(entity).get::<BuffsComponent>();
    let dropped: Vec<EntityBuffId> = registry
        .def(from)
        .passives
        .iter()
        .copied()
        .filter(|id| !now.contains(id))
        .filter(|&id| match buffs.and_then(|buffs| buffs.term(id)) {
            Some(Term::While(Held::Passive)) => true,
            Some(Term::While(Held::Applied))
            | Some(Term::Forever)
            | Some(Term::For { .. })
            | Some(Term::Upkeep { .. })
            | None => false,
        })
        .collect();
    for id in dropped {
        remove_entity_buff(world, entity, id);
    }
}

/// Ends each buff `entity` bears that the form `form` cannot carry: one
/// modifying a stat the form does not declare. The passives of the entity's
/// own form are left to [`shed_form`].
pub fn shed_unfit(world: &mut World, entity: Entity, form: EntityTypeId) {
    let registry = world.resource::<ContentRegistry>();
    let def = registry.def(form);
    let unfit: Vec<EntityBuffId> = match world.entity(entity).get::<BuffsComponent>() {
        Some(buffs) => buffs
            .active()
            .map(|(id, _)| id)
            .filter(|&id| match buffs.term(id) {
                Some(Term::While(Held::Passive)) => false,
                Some(Term::While(Held::Applied))
                | Some(Term::Forever)
                | Some(Term::For { .. })
                | Some(Term::Upkeep { .. }) => true,
                None => unreachable!("an active buff has a term"),
            })
            .filter(|&id| {
                !registry
                    .entity_buff_def(id)
                    .effects
                    .iter()
                    .all(|effect| match effect {
                        EntityEffect::Modifiers(modifiers) => modifiers
                            .modifiers()
                            .iter()
                            .all(|modifier| def.base_stat(modifier.stat).is_some()),
                        EntityEffect::Disable | EntityEffect::Conceal => true,
                    })
            })
            .collect(),
        None => Vec::new(),
    };
    for id in unfit {
        remove_entity_buff(world, entity, id);
    }
}

/// Ends each buff `entity` bears that holds `While` a requirement it no longer
/// meets, then applies each passive of its type it does not bear and whose
/// requirement it meets.
/// Player-scoped leaves are asked of the bearer's owner, and hold for none when
/// it has none.
pub fn refit_entity(world: &mut World, entity: Entity) {
    let owner = entity_def::owner(world, entity);
    for id in lapsed(world, owner, entity) {
        remove_entity_buff(world, entity, id);
    }
    for id in due(world, owner, entity) {
        fit_passive(world, entity, id);
    }
}

/// Fits the passive `id` onto `entity`, as [`apply_entity_buff`] applies a
/// buff, marked as borne of its type. Panics when `entity` cannot carry it.
fn fit_passive(world: &mut World, entity: Entity, id: EntityBuffId) {
    match bear(world, entity, id, Held::Passive) {
        Bearing::Borne => {}
        Bearing::Uncarried => panic!("fit_passive is given a passive its bearer can carry"),
    }
}

/// Puts the buff `id` on `entity`, a buff held on a requirement marked as
/// `held`, when the entity can carry it.
fn bear(world: &mut World, entity: Entity, id: EntityBuffId, held: Held) -> Bearing {
    if !entity_def::can_carry(world, entity, id) {
        return Bearing::Uncarried;
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
        Lasting::While(_) => Term::While(held),
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
    Bearing::Borne
}

/// The buffs `entity` bears that hold `While` a requirement it no longer
/// meets, in application order.
fn lapsed(world: &World, owner: Option<PlayerId>, entity: Entity) -> Vec<EntityBuffId> {
    let Some(buffs) = world.entity(entity).get::<BuffsComponent>() else {
        return Vec::new();
    };
    let registry = world.resource::<ContentRegistry>();
    buffs
        .active()
        .map(|(id, _)| id)
        .filter(|&id| match &registry.entity_buff_def(id).lasting {
            Lasting::While(requirement) => !requirements::met_by(world, owner, entity, requirement),
            Lasting::Forever | Lasting::For(_) | Lasting::Upkeep { .. } => false,
        })
        .collect()
}

/// The passives of `entity`'s type it does not bear and whose requirement it
/// meets, in the order the type names them.
fn due(world: &World, owner: Option<PlayerId>, entity: Entity) -> Vec<EntityBuffId> {
    let registry = world.resource::<ContentRegistry>();
    entity_def::of(world, entity)
        .passives
        .iter()
        .copied()
        .filter(|&id| !entity_def::bears(world, entity, id))
        .filter(|&id| match &registry.entity_buff_def(id).lasting {
            Lasting::While(requirement) => requirements::met_by(world, owner, entity, requirement),
            Lasting::Forever | Lasting::For(_) | Lasting::Upkeep { .. } => {
                unreachable!("the registry admits only passives that hold on a requirement")
            }
        })
        .collect()
}
