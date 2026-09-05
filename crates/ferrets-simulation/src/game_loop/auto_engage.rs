//! Idle defense: stance-driven target acquisition for entities with no orders.
//!
//! What is engaged here is the weapon a body points itself, by giving it an order
//! to fight with. The guns a body carries take their own quarrels in
//! [`super::turret`] whether it is idle or not, so a body that points no weapon of
//! its own is left alone here entirely.

use bevy_ecs::world::World;

use super::{acquire, attack_move};
use crate::{
    components::{
        entity_stats::StatsComponent,
        hidden::HiddenComponent,
        order_queue::OrderQueueComponent,
        stance::{Stance, StanceComponent},
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    order::{AttackTarget, Leash, Order},
    session::GameSession,
};
use ferrets_content::entity_stats::EntityStatId;

/// Engages idle entities per their stance, once per due tick (see
/// [`acquire::due`]).
///
/// - `Flee` and `HoldFire` never engage (fleeing is handled by
///   [`super::flee`]).
/// - `StandGround` attacks what its scan finds within weapon range; the
///   weapon-range leash breaks the fight off the moment the target leaves it,
///   so the unit never chases.
/// - `Defend` attacks within acquisition range, leashed to the spot it stands
///   on, and queues an attack-move back to that spot — the way home re-engages
///   anything it meets.
///
/// Only entities with an empty order queue engage: a unit executing orders is
/// never hijacked.
pub fn tick(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();

    for (id, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !acquire::due(id, tick) {
            continue;
        }
        let stance = match world.entity(entity).get::<StanceComponent>() {
            Some(StanceComponent(stance)) => *stance,
            None => continue,
        };
        if !stance.auto_engages() {
            continue;
        }
        if world.entity(entity).contains::<HiddenComponent>() {
            continue;
        }
        // Only an operating body takes the initiative: a site still going
        // up and a disabled one stand idle.
        match entity_def::operation(world, entity) {
            Operation::Operating => {}
            Operation::UnderConstruction | Operation::Disabled => continue,
        }
        let entity_ref = world.entity(entity);
        if entity_ref
            .get::<OrderQueueComponent>()
            .is_none_or(|queue| queue.front().is_some())
        {
            continue;
        }
        // What is engaged here is the weapon a body points itself: an unarmed body
        // has nothing to engage with, and one that fights only from turrets has
        // nothing an order would work — an order binds every gun it carries to one
        // target, and guns that pick their own are the whole reason to carry
        // several. Those hunt for themselves in [`super::turret`].
        if entity_def::of(world, entity).attack.is_none() {
            continue;
        }

        let range = entity_ref
            .get::<StatsComponent>()
            .and_then(|stats| stats.effective_as_u32(EntityStatId::ATTACK_RANGE))
            .expect("attackers have a range stat");
        let anchor = entity_def::position(world, entity);

        match stance {
            Stance::StandGround => {
                if let Some(target) =
                    // What its own weapon reaches: this order is that weapon's
                    // fight, and its turrets pick their own quarrels.
                    acquire::find_target(
                        world,
                        entity,
                        entity_def::body_weapon_targets(world, entity),
                        range,
                    )
                    && let Some(mut queue) =
                        world.entity_mut(entity).get_mut::<OrderQueueComponent>()
                {
                    queue.push(
                        Order::Attack {
                            target: AttackTarget::Entity(target),
                            leash: Some(Leash {
                                anchor,
                                radius: range,
                            }),
                        },
                        None,
                    );
                }
            }
            Stance::Defend => {
                // Its own weapon's fight, so its own weapon's notice: the wider
                // reach a turret names is that turret's own business.
                let notice = world
                    .entity(entity)
                    .get::<StatsComponent>()
                    .and_then(|stats| stats.effective_as_u32(EntityStatId::ACQUIRE_RANGE))
                    .expect("attackers have an acquisition range stat");
                if let Some(attack) = attack_move::engagement(
                    world,
                    entity,
                    entity_def::body_weapon_targets(world, entity),
                    notice,
                ) && let Some(mut queue) =
                    world.entity_mut(entity).get_mut::<OrderQueueComponent>()
                {
                    queue.push(attack, None);
                    queue.push(Order::AttackMove { target: anchor }, None);
                }
            }
            Stance::Flee | Stance::HoldFire => unreachable!("filtered above"),
        }
    }
}
