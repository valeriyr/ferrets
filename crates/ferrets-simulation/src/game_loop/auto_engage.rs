//! Idle defense: stance-driven target acquisition for entities with no orders.

use bevy_ecs::world::World;

use super::{acquire, attack_move};
use crate::{
    components::{
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::OrderQueueComponent,
        stance::{Stance, StanceComponent},
        stats::StatsComponent,
    },
    content::stats::StatId,
    entity_def,
    entity_index::EntityIndex,
    order::{AttackTarget, Leash, Order},
    session::GameSession,
};

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
        let entity_ref = world.entity(entity);
        if entity_ref
            .get::<OrderQueueComponent>()
            .is_none_or(|queue| queue.front().is_some())
        {
            continue;
        }
        if !entity_def::of(world, entity).can_attack() {
            continue;
        }

        let range = entity_ref
            .get::<StatsComponent>()
            .and_then(|stats| stats.effective_as_u32(StatId::ATTACK_RANGE))
            .expect("attackers have a range stat");
        let anchor = entity_ref.get::<LocationComponent>().unwrap().position;

        match stance {
            Stance::StandGround => {
                if let Some(target) = acquire::find_target(world, entity, range)
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
                if let Some(attack) = attack_move::engagement(world, entity)
                    && let Some(mut queue) =
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
