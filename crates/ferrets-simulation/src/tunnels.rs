//! Tunnel ends and their bindings: an end raised by an unbound end of the same
//! tunnel is bound to it from birth.

use bevy_ecs::{
    entity::Entity,
    world::{Mut, World},
};
use ferrets_content::{
    entity_type_def::EntityTypeDef,
    registry::ContentRegistry,
    tunnel::{OtherEndLost, TunnelId},
};

use crate::{
    bore_id::{BoreId, BoreIds},
    components::tunnel::{OtherEnd, TunnelEndComponent},
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SpawnCause},
    simulation_id::SimulationId,
    spawn,
};

/// Binds the newborn `born` to the end that produced it, as `cause` names it,
/// when both are ends of one tunnel and the producer is unbound; `born` takes
/// its bore. Anything else is left as it was born.
pub(crate) fn bind_at_birth(world: &mut World, born: Entity, cause: SpawnCause) {
    let producer = match cause {
        SpawnCause::Trained { trainer } => trainer,
        SpawnCause::Warped { warper } => warper,
        SpawnCause::Founded { builder } => builder,
        SpawnCause::Summoned { by } | SpawnCause::Raised { by, .. } => by,
        SpawnCause::Placed
        | SpawnCause::Sandbox
        | SpawnCause::Bred { .. }
        | SpawnCause::Bequeathed { .. }
        | SpawnCause::Uncovered { .. } => return,
    };
    let Some(tunnel) = tunnel_of(world, born) else {
        return;
    };
    let raiser = world
        .resource::<EntityIndex>()
        .alive(producer)
        .expect("a birth's producer is alive at the birth");
    if tunnel_of(world, raiser) != Some(tunnel) {
        return;
    }
    let raiser_end = end_state(world, raiser);
    let bore = match raiser_end.other {
        OtherEnd::Unbound => raiser_end.bore,
        // A second end queued while the raiser was unbound is born after the
        // first bound it, and stays unbound.
        OtherEnd::Bound(_) => return,
    };
    let born_id = entity_def::simulation_id(world, born);
    bind(world, raiser, born_id, bore);
    bind(world, born, producer, bore);
}

/// Why an end is lost to its tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndLoss {
    /// It died, or changed into a form that is no end of the tunnel.
    Gone,
    /// It changed hands.
    Captured,
}

/// Unbinds the tunnel end `lost`, lost to its tunnel as `loss` says, from its
/// other end, which then fares as the tunnel's `other_end_lost` says: razed,
/// or standing unbound with the bore. A captured end and an end lost while
/// rising always leave the other end standing. An unbound end has no other
/// end to fare.
///
/// Panics for an entity that is no tunnel end.
pub(crate) fn end_lost(world: &mut World, lost: Entity, loss: EndLoss) {
    let tunnel = tunnel_of(world, lost).expect("only a tunnel end is lost to its tunnel");
    let other_id = match end_state(world, lost).other {
        OtherEnd::Bound(other) => other,
        OtherEnd::Unbound => return,
    };
    let other = world
        .resource::<EntityIndex>()
        .alive(other_id)
        .expect("a bound end's other end is alive");
    end_state_mut(world, lost).other = OtherEnd::Unbound;
    end_state_mut(world, other).other = OtherEnd::Unbound;
    let fares = match (loss, entity_def::rising(world, lost)) {
        (EndLoss::Captured, _) | (EndLoss::Gone, true) => OtherEndLost::Stands,
        (EndLoss::Gone, false) => world
            .resource::<ContentRegistry>()
            .tunnel_def(tunnel)
            .other_end_lost(),
    };
    match fares {
        OtherEndLost::Stands => {}
        OtherEndLost::Razed => spawn::despawn_entity(world, other, DeathCause::Decayed),
    }
}

/// Starts the tunnel end `end` on a newly minted bore, as an end born
/// unbound.
///
/// Panics for an entity that is no tunnel end.
pub(crate) fn bore_anew(world: &mut World, end: Entity) {
    let bore = world.resource_mut::<BoreIds>().mint();
    end_state_mut(world, end).bore = bore;
}

/// Whether `producer` may not raise one of `def`: it is a bound end of the
/// tunnel `def` is an end of.
pub fn refuses_raising(world: &World, producer: Entity, def: &EntityTypeDef) -> bool {
    let Some(tunnel) = def.gateway().and_then(|gateway| gateway.joins().tunnel()) else {
        return false;
    };
    if tunnel_of(world, producer) != Some(tunnel) {
        return false;
    }
    match end_state(world, producer).other {
        OtherEnd::Bound(_) => true,
        OtherEnd::Unbound => false,
    }
}

/// The other end the tunnel end `end` is bound to, or `None` for an unbound
/// end.
///
/// Panics for an entity that is no tunnel end.
pub fn other_end(world: &World, end: Entity) -> Option<Entity> {
    match end_state(world, end).other {
        OtherEnd::Bound(other) => Some(
            world
                .resource::<EntityIndex>()
                .alive(other)
                .expect("a bound end's other end is alive"),
        ),
        OtherEnd::Unbound => None,
    }
}

/// The tunnel `entity` is an end of, or `None` for anything else.
fn tunnel_of(world: &World, entity: Entity) -> Option<TunnelId> {
    entity_def::of(world, entity)
        .gateway()
        .and_then(|gateway| gateway.joins().tunnel())
}

/// Binds the tunnel end `end` to `other` on `bore`.
fn bind(world: &mut World, end: Entity, other: SimulationId, bore: BoreId) {
    *end_state_mut(world, end) = TunnelEndComponent {
        other: OtherEnd::Bound(other),
        bore,
    };
}

/// The tunnel end `end`'s state.
///
/// Panics for an entity that is no tunnel end.
fn end_state(world: &World, end: Entity) -> &TunnelEndComponent {
    world
        .entity(end)
        .get::<TunnelEndComponent>()
        .expect("a tunnel end keeps its state")
}

/// The tunnel end `end`'s state, to change.
///
/// Panics for an entity that is no tunnel end.
fn end_state_mut(world: &mut World, end: Entity) -> Mut<'_, TunnelEndComponent> {
    world
        .entity_mut(end)
        .into_mut::<TunnelEndComponent>()
        .expect("a tunnel end keeps its state")
}
