//! What a gateway does with a body that enters it: puts it in its link,
//! or sets it down within the other end of its tunnel's unload range.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    affiliation::Affiliation,
    entity_stats::EntityStatId,
    gateway::{GatewayDef, Joins},
    kinds::Kinds,
    landing::Landing,
    registry::ContentRegistry,
    tunnel::Passage,
};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::{
    chase,
    orders::{self, Refusal},
};
use crate::{
    beside::Lean,
    components::{
        gateway::GatewayComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner,
        rally::RallyTarget,
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    landing::{self, Bound, ComesFrom, Lander, Landings},
    links::{self, LinkKey, LinkPassengers, Membership},
    make_way::{self, Asked},
    rally,
    session::GameSession,
    simulation_id::SimulationId,
    spawn::{self, SetDown},
    tunnels,
};

/// What entering a gateway does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// The body goes into this link.
    Held(LinkKey),
    /// The body steps out within this other end's unload range at once.
    Crosses(Entity),
}

/// Whether `entity` may enter `gateway` now, and what entering does: the
/// gateway is [open as a door](open_as_door) with doors that let bodies in,
/// its link admits `entity`'s owner and type, and a holding link has room for
/// it while a crossing tunnel's other end is open as a door with doors that
/// let bodies out (else [`Refusal::TargetUnfit`]).
pub(crate) fn would_enter(
    world: &World,
    entity: Entity,
    gateway: Entity,
) -> Result<Entry, Refusal> {
    let def = gateway_def(world, gateway);
    if !open_as_door(world, gateway) || !def.doors().admits_entrance() {
        return Err(Refusal::TargetUnfit);
    }
    let registry = world.resource::<ContentRegistry>();
    let (carries, boarding, passage): (&Kinds, Affiliation, Passage) = match def.joins() {
        Joins::Network(network) => {
            let network = registry.network_def(network);
            (
                network.carries(),
                network.boarding(),
                Passage::Held {
                    capacity: network.capacity(),
                    fate: network.fate(),
                },
            )
        }
        Joins::Tunnel(tunnel) => {
            let tunnel = registry.tunnel_def(tunnel);
            (tunnel.carries(), tunnel.boarding(), tunnel.passage())
        }
    };
    let allowed = owner::admits(
        world.resource::<GameSession>(),
        boarding,
        entity_def::owner(world, gateway),
        entity_def::owner(world, entity),
    );
    if !allowed || !carries.admits(entity_def::of(world, entity)) {
        return Err(Refusal::TargetUnfit);
    }
    match passage {
        Passage::Held { capacity, .. } => {
            let key = match links::membership_of(world, gateway) {
                Membership::Joined(key) => key,
                Membership::Apart => {
                    unreachable!("an ownerless gateway of a network scoped by owner admits nobody")
                }
            };
            if held_cargo(world, key) + entity_def::cargo_size(world, entity) > capacity {
                return Err(Refusal::TargetUnfit);
            }
            Ok(Entry::Held(key))
        }
        Passage::Crosses => {
            let Some(other) = tunnels::other_end(world, gateway) else {
                return Err(Refusal::TargetUnfit);
            };
            if !open_as_door(world, other) || !gateway_def(world, other).doors().admits_exit() {
                return Err(Refusal::TargetUnfit);
            }
            Ok(Entry::Crosses(other))
        }
    }
}

/// Where a body that enters goes, settled before anything moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Through {
    /// Into this link.
    Into(LinkKey),
    /// Onto this cell within this other end's unload range.
    Across {
        /// The other end it steps out of.
        other: Entity,
        /// The anchor it steps out on.
        cell: CellPos,
    },
}

/// What came of taking a body through a gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Intake {
    /// It went this way.
    Taken(Through),
    /// No cell within the other end's unload range takes it now; the body
    /// was not taken in, and own and allied movers keeping it out were asked
    /// off.
    Waits,
}

/// Takes `body`, judged this tick, through `gateway`: whatever it was doing is
/// soft-canceled first, then it turns to the door and goes into the link or
/// across to the other end, unless no cell within the other end's unload
/// range takes it now.
pub(crate) fn take_in(world: &mut World, gateway: Entity, body: Entity) -> Intake {
    let entry = would_enter(world, body, gateway)
        .unwrap_or_else(|refusal| unreachable!("judged this tick, got {refusal:?}"));
    let Some(through) = way_through(world, body, entry) else {
        // Arrivals standing at the other end make way for those behind them:
        // own and allied movers that alone hold its ground are asked off.
        match entry {
            Entry::Crosses(other) => {
                let lifted: Vec<_> = entity_def::grid_mark(world, body).into_iter().collect();
                match make_way::clear_landing(
                    world,
                    entity_def::owner(world, body),
                    |world| other_end_lander(world, body, other),
                    &lifted,
                    &[body],
                ) {
                    Asked::Leaves | Asked::Boxed => {}
                }
            }
            Entry::Held(_) => {}
        }
        return Intake::Waits;
    };
    // A body taken in by the gateway's own order stops what it was doing; a
    // body boarding on its own initiative has its queue out of the world here.
    if world.entity(body).contains::<OrderQueueComponent>() {
        orders::drop_all(world, body, CancelPolicy::Soft);
        let body_id = entity_def::simulation_id(world, body);
        assert!(
            world.resource::<EntityIndex>().alive(body_id).is_some(),
            "a soft cancel kills nothing on a body judged not to be changing form"
        );
    }
    chase::face_entity(world, body, gateway);
    enter(world, gateway, body, through);
    Intake::Taken(through)
}

/// The earliest tick `gateway` may let its next body in at.
pub(crate) fn boarding_ready_at(world: &World, gateway: Entity) -> u32 {
    world
        .entity(gateway)
        .get::<GatewayComponent>()
        .expect("a gateway paces its entrance")
        .boarding_ready_at
}

/// Whether `gateway` lets bodies out of its link: it is one of a link, it is
/// [open as a door](open_as_door), its doors let bodies out, and its link
/// holds.
pub(crate) fn lets_out(world: &World, gateway: Entity) -> bool {
    match links::membership_of(world, gateway) {
        Membership::Joined(_) => {}
        Membership::Apart => return false,
    }
    let def = gateway_def(world, gateway);
    world.resource::<ContentRegistry>().joins_hold(def.joins())
        && def.doors().admits_exit()
        && open_as_door(world, gateway)
}

/// The first body inside `gateway`'s link by id, past `after` when given, or
/// `None` for a gateway of no link.
pub(crate) fn next_held(
    world: &World,
    gateway: Entity,
    after: Option<SimulationId>,
) -> Option<SimulationId> {
    links::passengers_of(world, gateway).find(|&id| after.is_none_or(|after| id > after))
}

/// Sets `body` down out of `gateway`'s link within the gateway's
/// `unload_range`: on the free cell nearest `at` when the order names a point,
/// else nearest the gateway as `lean` says. Out, it leaves the link.
pub(crate) fn let_out(
    world: &mut World,
    gateway: Entity,
    body: Entity,
    at: Option<FixedUVec2>,
    lean: Lean,
) -> SetDown {
    let gateway_id = entity_def::simulation_id(world, gateway);
    let set_down = match at {
        Some(at) => spawn::place_back_near(world, body, CellPos::from(at), gateway_id),
        None => spawn::place_back_from_holder(world, body, gateway_id, lean),
    };
    match set_down {
        SetDown::Placed => {
            let key = joined_key(world, gateway);
            let body_id = entity_def::simulation_id(world, body);
            world.resource_mut::<LinkPassengers>().remove(key, body_id);
        }
        SetDown::NoCell => {}
    }
    set_down
}

/// Takes `body`, gone from the world, out of `gateway`'s link.
pub(crate) fn drop_stale(world: &mut World, gateway: Entity, body: SimulationId) {
    let key = joined_key(world, gateway);
    world.resource_mut::<LinkPassengers>().remove(key, body);
}

/// Lets `entity` through `gateway` as `through` says — into its link, or onto
/// its cell out of the other end, due the other end's rally unless it names
/// either end — and starts the gateway's pace.
fn enter(world: &mut World, gateway: Entity, entity: Entity, through: Through) {
    match through {
        Through::Into(key) => spawn::put_into_link(world, entity, key),
        Through::Across { other, cell } => {
            spawn::relocate(world, &[(entity, cell)]);
            let ends = [
                entity_def::simulation_id(world, gateway),
                entity_def::simulation_id(world, other),
            ];
            match rally::target_of(world, other) {
                Some(RallyTarget::Entity(id)) if ends.contains(&id) => {}
                Some(RallyTarget::Entity(_) | RallyTarget::Position(_)) | None => {
                    rally::mark_rally_due(world, other, entity)
                }
            }
        }
    }
    let tick = world.resource::<GameSession>().tick();
    let period = entity_def::effective_ticks(world, gateway, EntityStatId::LOAD_PERIOD);
    world
        .entity_mut(gateway)
        .get_mut::<GatewayComponent>()
        .expect("a gateway paces its entrance")
        .boarding_ready_at = tick + period;
}

/// Where `entity` goes as `entry` says, or `None` when no cell within the
/// other end's unload range takes it now.
fn way_through(world: &World, entity: Entity, entry: Entry) -> Option<Through> {
    match entry {
        Entry::Held(key) => Some(Through::Into(key)),
        Entry::Crosses(other) => {
            out_of_other_end(world, entity, other).map(|cell| Through::Across { other, cell })
        }
    }
}

/// The cargo size of every body in `key`.
fn held_cargo(world: &World, key: LinkKey) -> u32 {
    let index = world.resource::<EntityIndex>();
    world
        .resource::<LinkPassengers>()
        .passengers(key)
        .filter_map(|id| index.alive(id))
        .map(|body| entity_def::cargo_size(world, body))
        .sum()
}

/// The anchor within `other`'s unload range that `entity` steps out on,
/// nearest the other end and then where it stood, or `None` when nothing in
/// range takes `entity` now.
fn out_of_other_end(world: &World, entity: Entity, other: Entity) -> Option<CellPos> {
    let owner = entity_def::owner(world, entity).expect("a body that enters a gateway is owned");
    let lifted: Vec<_> = entity_def::grid_mark(world, entity).into_iter().collect();
    let ignored = [entity];
    let mut landings = Landings::new(owner, Landing::Anywhere, &[], &lifted, &ignored);
    landing::land(world, &mut landings, other_end_lander(world, entity, other))
}

/// How `entity` steps out of `other`: within its unload range, nearest it and
/// then where `entity` stood.
fn other_end_lander(world: &World, entity: Entity, other: Entity) -> Lander<'_> {
    let rect = entity_def::standing_rect(world, other);
    let range = entity_def::effective_stat_u32(world, other, EntityStatId::UNLOAD_RANGE);
    Lander {
        def: entity_def::of(world, entity),
        owner: entity_def::owner(world, entity),
        from: ComesFrom::within(world, other, range),
        bound: Bound::Range { rect, range },
        around: rect,
        toward: entity_def::footprint_rect(world, entity),
    }
}

/// `gateway`'s definition as a gateway.
fn gateway_def(world: &World, gateway: Entity) -> GatewayDef {
    *entity_def::of(world, gateway)
        .gateway()
        .expect("only a gateway lets bodies through")
}

/// The link `gateway`, which somebody is inside, is one of.
///
/// Panics for a gateway of no link: nobody is ever inside one.
fn joined_key(world: &World, gateway: Entity) -> LinkKey {
    match links::membership_of(world, gateway) {
        Membership::Joined(key) => key,
        Membership::Apart => panic!("only a gateway of a link has anybody inside"),
    }
}

/// Whether `gateway` is open as a door now: it stands on the map, operating,
/// and changes no form it wears from the start. One changing through an
/// interim form is open as that form's gateway.
fn open_as_door(world: &World, gateway: Entity) -> bool {
    let operating = match entity_def::operation(world, gateway) {
        Operation::Operating => true,
        Operation::Rising | Operation::Disabled(_) | Operation::Stranded => false,
    };
    operating
        && !entity_def::hidden(world, gateway)
        && !entity_def::changing_in_origin(world, gateway)
}
