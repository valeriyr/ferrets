//! Warp-in: a warper sets a unit down at once on a cell its player picks,
//! rising unattended over the unit's own rise time, and cools down for the
//! unit's train time.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{price::Price, registry::ContentRegistry};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::orders::{self, Refusal};
use crate::{
    components::{
        location::LocationComponent,
        rise::{RiseAttendance, RiseOrigin, RisingComponent},
        warp::WarpComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SpendCause},
    fields, ground, landing,
    map::OccupancyClass,
    overbuild::{self, Cover},
    requirements,
    resources::{self, PlayerResources},
    session::{GameSession, player_id::PlayerId},
    spawn::{self, FieldReach},
    supply, tunnels,
};

/// Why a warp-in is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpRefusal {
    /// The entity warps nothing in, or not the type.
    Incapable,
    /// The warper is not working: the order gate's refusal.
    Warper(Refusal),
    /// The warper is changing form.
    Busy,
    /// The warper has not cooled down from its last warp-in.
    CoolingDown,
    /// A bound tunnel end has its other end already.
    Bound,
    /// The unit's requirements are unmet.
    Unmet,
    /// The player's supply has no room for it.
    NoSupply,
    /// The player cannot pay its price.
    Unaffordable,
    /// The ground does not take its footprint at the cell, or a type raised
    /// over a source finds none of its type exactly there.
    Blocked,
    /// The player's side does not know the cells as the landing asks.
    Unseen,
    /// The cells break the warper's or the unit's field rules.
    OutsideField,
}

/// A warp-in judged to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarpPlan {
    /// The warper warping it in.
    warper: Entity,
    /// The unit's type, by name.
    type_name: String,
    /// The anchor it rises on.
    anchor: CellPos,
    /// What it costs.
    price: Price,
    /// Ticks the warper cools down for.
    cooldown: u32,
    /// What it is set down on.
    cover: Cover,
}

/// Whether `player`'s `warper` may warp in a unit of `type_name` anchored at
/// `at` now, judged without touching the world: the warper and the unit as
/// [`judge_warper`] says, and the unit's footprint at `at` ground that takes
/// it — for a type raised over a source, over a source of that type standing
/// exactly on it, judged as though the source were off the grid — known as
/// the warper's landing asks, inside the warper's field rules and the unit's
/// own.
pub fn judge(
    world: &World,
    player: PlayerId,
    warper: Entity,
    type_name: &str,
    at: FixedUVec2,
) -> Result<WarpPlan, WarpRefusal> {
    judge_warper(world, player, warper, type_name)?;
    let warper_def = entity_def::warper_def(world, warper).expect("judged an entity that warps");
    let unit = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("a warper warps in only registered types");
    let anchor = CellPos::from(at);
    let location_def = unit.location.expect("a warped unit stands somewhere");
    let footprint = CellRect::new(anchor, location_def.size());
    let cover = match unit.overbuilds.as_deref() {
        Some(over) => Cover::Source(
            overbuild::source_under(world, over, footprint).ok_or(WarpRefusal::Blocked)?,
        ),
        None => Cover::Nothing,
    };
    let (lifted, ignored) = overbuild::set_aside(world, cover);
    let placed = LocationComponent::new(FixedUVec2::from(anchor), spawn::DEFAULT_FACING);
    if !ground::can_place(
        world,
        &placed,
        location_def,
        OccupancyClass::of(unit),
        &lifted,
        &ignored,
    ) {
        return Err(WarpRefusal::Blocked);
    }
    if !landing::sees(world, player, warper_def.landing(), footprint) {
        return Err(WarpRefusal::Unseen);
    }
    if !fields::rules_allow(world, Some(player), warper_def.within(), footprint)
        || !fields::allows_placement(world, Some(player), unit, anchor)
    {
        return Err(WarpRefusal::OutsideField);
    }
    Ok(WarpPlan {
        warper,
        type_name: type_name.to_string(),
        anchor,
        price: unit.price.clone(),
        cooldown: unit
            .train_time
            .expect("a warped unit declares its train time"),
        cover,
    })
}

/// Whether `player`'s `warper` may warp in a unit of `type_name` somewhere
/// now: it warps the type in, operates and keeps its form, has cooled down
/// and raises no second tunnel end; and the unit's requirements, supply and
/// price are met.
pub fn judge_warper(
    world: &World,
    player: PlayerId,
    warper: Entity,
    type_name: &str,
) -> Result<(), WarpRefusal> {
    if !entity_def::warper_def(world, warper).is_some_and(|def| def.can_warp(type_name)) {
        return Err(WarpRefusal::Incapable);
    }
    orders::requires_operating(world, warper).map_err(WarpRefusal::Warper)?;
    if entity_def::changing(world, warper) {
        return Err(WarpRefusal::Busy);
    }
    let tick = world.resource::<GameSession>().tick();
    if tick < ready_at(world, warper) {
        return Err(WarpRefusal::CoolingDown);
    }
    let unit = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("a warper warps in only registered types");
    if tunnels::refuses_raising(world, warper, unit) {
        return Err(WarpRefusal::Bound);
    }
    if !requirements::met(world, player, Some(warper), &unit.requires) {
        return Err(WarpRefusal::Unmet);
    }
    if !supply::allows(world, player, unit) {
        return Err(WarpRefusal::NoSupply);
    }
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, &unit.price)
    {
        return Err(WarpRefusal::Unaffordable);
    }
    Ok(())
}

/// Warps in what `plan` judged for `player`: pays the price, sets the unit
/// down rising unattended for its warper, and starts the warper's cooldown.
pub fn begin(world: &mut World, player: PlayerId, plan: WarpPlan) {
    let warper_id = entity_def::simulation_id(world, plan.warper);
    let ready_at = world.resource::<GameSession>().tick() + plan.cooldown;
    resources::charge(
        world,
        player,
        plan.price,
        SpendCause::Warp { warper: warper_id },
    );
    // A source the unit is raised over comes off the grid so the unit takes
    // its cells, and is covered once the unit stands on them.
    overbuild::lift(world, plan.cover);
    let (unit, _) = spawn::spawn_warped(
        world,
        &plan.type_name,
        FixedUVec2::from(plan.anchor),
        player,
        FieldReach::Initial,
        warper_id,
        ready_at,
    )
    .unwrap_or_else(|| unreachable!("the ground was judged to take it this tick"));
    overbuild::settle(world, plan.cover, Some(unit));
    world
        .entity_mut(plan.warper)
        .get_mut::<WarpComponent>()
        .expect("a warper keeps its cooldown")
        .ready_at = ready_at;
}

/// Tears down `unit`, still warping in, pays its price back to the player
/// that paid it as a warp called off at the warper that warped it in, and
/// resets that warper's cooldown when this warp-in is what it cools down
/// from and the payer still owns it. Panics when `unit` is not warping in.
pub(crate) fn call_off(world: &mut World, unit: Entity) {
    let rising = world
        .get::<RisingComponent>(unit)
        .expect("a warp called off is still rising");
    let (payer, warp_ready_at) = match rising.origin {
        RiseOrigin::Warped {
            payer, ready_at, ..
        } => (payer, ready_at),
        RiseOrigin::Founded { .. } | RiseOrigin::Summoned { .. } => {
            panic!("a warp called off is of a unit warped in")
        }
    };
    let warper = match rising.attendance {
        RiseAttendance::Unattended { founder } => founder,
        RiseAttendance::Crew { .. } | RiseAttendance::Halted => {
            unreachable!("a unit warping in rises unattended")
        }
    };
    let price = entity_def::of(world, unit).price.clone();
    spawn::despawn_entity(world, unit, DeathCause::Canceled);
    resources::refund(world, payer, price, SpendCause::Warp { warper });
    let tick = world.resource::<GameSession>().tick();
    // A later warp-in's cooldown is that one's to reset; a warper gone,
    // handed to another player, or changed into a form that does not warp
    // keeps no cooldown of the payer's to reset.
    let warper_entity = world
        .resource::<EntityIndex>()
        .alive(warper)
        .filter(|&entity| entity_def::owner(world, entity) == Some(payer));
    if let Some(mut warp) = warper_entity.and_then(|entity| world.get_mut::<WarpComponent>(entity))
        && warp.ready_at == warp_ready_at
    {
        warp.ready_at = tick;
    }
}

/// The earliest tick `warper` may warp in its next unit at.
fn ready_at(world: &World, warper: Entity) -> u32 {
    world
        .entity(warper)
        .get::<WarpComponent>()
        .expect("a warper keeps its cooldown")
        .ready_at
}
