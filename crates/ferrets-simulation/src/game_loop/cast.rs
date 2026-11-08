//! Casting: the Cast order an entity works through, and the casts that belong
//! to a player rather than to anything on the map.
//!
//! An entity's cast is an order like any other — queued, canceled and
//! replaced like a walk — which walks the caster into the skill's reach first
//! when it declares one, and lands where it stands when it does not; a
//! transient cast is no order, and lands as the command arrives, beside
//! whatever the caster is doing. A player's cast has no caster to send
//! anywhere, so it happens where the command arrives.
//!
//! The order half is called by [`super::orders`] as part of the shared order
//! lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    attack::Slain,
    entity_buffs::Interruption,
    entity_type_def::EntityTypeId,
    field::FieldPlacement,
    kinds::Kinds,
    landing::Landing,
    pool_def::PoolId,
    price::Price,
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{
        AimParty, CasterParty, Casting, Count, EntityCastEffect, EntityCastTarget, FromAimTo,
        FromCasterTo, GatherSight, Gathering, Journey, PaymentPoint, PlayerCastEffect, Reach,
        Refund, Shortfall, SkillCaster, SkillDef, SkillId, SummonArrival, SummonPlacement, Whom,
    },
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::{
    buffs::{self, Bearing},
    chase::{self, Destination},
    cost, damage,
    orders::{self, Processing, Refusal},
};
use crate::{
    command::SkillTarget,
    components::{
        cast::{CastComponent, CastStage, Paid},
        entity_skills::{self, SkillsComponent},
        order_queue::{CancelPolicy, OrderState},
        owner, pools,
        rise::{RiseAttendance, RiseOrigin},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{EventRecord, SimulationEvent, SpendCause},
    game_loop::fields,
    landing::{self, Bound, ComesFrom, Lander, Landings},
    map::Map,
    order::Order,
    overbuild::{self, Cover},
    player_skills::{self, PlayerSkills},
    reach, requirements,
    resources::{self, PlayerResources},
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
    spawn::{self, Arrival, FieldReach},
    supply, tunnels,
    visibility::{self, Senses},
    watches,
};

/// Whether `entity` may start this Cast: it operates, its type declares the
/// skill, and the requirements its owner must meet are met.
///
/// The requirements are judged again once casting starts, a walk later, and
/// as the cast lands when it pays then; whether the aim is still there and
/// whether the cost can be paid are settled once casting starts. The cooldown
/// is judged as the cast is ordered (see [`judge_issue`]).
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let (skill, _) = order.cast_params().expect("Cast order must have a skill");
    startable(world, entity, skill)
}

/// Called once when a Cast order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    world.entity_mut(entity).insert(CastComponent::starting());
    OrderState::InProcessing
}

/// Called when a Cast order resumes from `Suspended` (its walk toward the aim
/// just finished). The driver component survives suspension; what the aim is
/// doing by now is judged in [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Cast entry that has a cancel policy.
///
/// The cast simply stops. Called off before it lands, a cast that paid at its
/// start gives back what its terms say; called off after, the cast has already
/// landed and what it took is gone — the caster is only cut free of the rest
/// of its own cast.
pub fn cancel_processing(
    entity: Entity,
    order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> Processing {
    if let Some(state) = world.entity_mut(entity).take::<CastComponent>() {
        let (skill, _) = order.cast_params().expect("Cast order must have a skill");
        match state.stage {
            CastStage::Working => give_back(world, entity, skill, state.paid),
            CastStage::Holding => {}
        }
    }
    Processing::state(OrderState::Finished)
}

/// Whether a Cast can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance a Cast order by one tick.
///
/// Walk to within the skill's reach of the aim (suspending on a chase move),
/// then cast: the first tick of casting pays what the cast pays at its start,
/// the effect lands on the tick its point names, and the caster stands through
/// the rest of its period before the order finishes. Until it lands, a tick
/// its aim cannot be held — gone, no longer seen, out of reach once casting
/// has started or of a caster that may not close (see
/// [`entity_def::may_close`]), or, until casting starts, an aim the buff it
/// applies would not hold on — cuts the cast short; a caster that stops operating has its
/// cast canceled by the order loop before this runs.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let (skill, target) = order.cast_params().expect("Cast order must have a skill");
    let Some(mut state) = world.entity_mut(entity).take::<CastComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let Some(player) = entity_def::owner(world, entity) else {
        return Processing::state(OrderState::Finished);
    };
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a declared skill id comes from the registry")
        .clone();

    let (point, period) = casting(world, entity, &def);

    match state.stage {
        CastStage::Working => {
            let started = state.phase > 0;
            let Ok(aim) = resolve(world, player, entity, skill, target) else {
                return cut_short(world, entity, skill, state.paid);
            };
            // Whether the buff would hold is judged until the cast begins;
            // once begun, a cast paid at its start is not judged again.
            if !started && holds(world, skill, aim).is_err() {
                return cut_short(world, entity, skill, state.paid);
            }
            // A caster that may not close casts only on what it reaches from
            // where it stands, as the cast was judged when ordered.
            if !reaches_aim(world, entity, skill, aim) {
                return cut_short(world, entity, skill, state.paid);
            }
            // Closing on the aim is the work before casting starts. A skill
            // that declares no reach lands from wherever the caster stands, so
            // there is nothing to close on and nothing to turn toward.
            if let Some(range) = reach_of(world, entity, &def) {
                let (goal_position, goal_size) = aimed_footprint(world, aim);
                let destination = match aim {
                    CastAim::Entity(target) | CastAim::Remains(target) => chase::advance_to_entity(
                        &mut state.last_chase,
                        world,
                        entity,
                        target,
                        range,
                    ),
                    CastAim::Cell(_) => chase::advance_to_point(
                        &mut state.last_chase,
                        world,
                        entity,
                        goal_position,
                        goal_size,
                        range,
                    ),
                };
                match destination {
                    Destination::OutOfReach => {
                        return cut_short(world, entity, skill, state.paid);
                    }
                    // A cast under way does not follow its aim: the aim that
                    // left its reach is out of it.
                    Destination::Walk(_) if started => {
                        return cut_short(world, entity, skill, state.paid);
                    }
                    Destination::Walk(move_order) => {
                        world.entity_mut(entity).insert(state);
                        return Processing::suspend(move_order);
                    }
                    Destination::Arrived => {}
                }
                chase::face(world, entity, goal_position, goal_size);
            }
            // The caster is where it needs to be: its first tick of casting
            // pays what the cast pays at its start, or ends the order unpaid.
            if !started {
                if !ready(world, entity, skill) {
                    return Processing::state(OrderState::Finished);
                }
                match start(world, player, entity, skill, &def, aim) {
                    CastStart::Started(paid) => state.paid = paid,
                    CastStart::Refused => return Processing::state(OrderState::Finished),
                }
            }
            state.phase += 1;
            // At its point or past it, rather than exactly at it: a point read
            // from a stat can shrink under a phase already worked. What has
            // landed is remembered, so it lands once however the numbers move.
            if state.phase < point {
                world.entity_mut(entity).insert(state);
                return Processing::state(OrderState::InProcessing);
            }
            state.stage = CastStage::Holding;
            match land(world, player, entity, skill, &def, aim, state.paid) {
                // Landed or refused, the caster stands through the rest of
                // the cast.
                CastLanding::Landed | CastLanding::Refused => {}
            }
            // A cast the caster did not survive — its own damage, or a body it
            // stood on being spent — ends here: the queue is out of the world
            // while an order runs, so the loop is what hands the caster its
            // Die order.
            if entity_def::dying(world, entity) {
                return Processing::finished_dying();
            }
        }
        // Past its point, the caster is standing through what is left of the
        // cast; the aim it landed on may well be gone by now.
        CastStage::Holding => state.phase += 1,
    }
    if state.phase < period {
        world.entity_mut(entity).insert(state);
        return Processing::state(OrderState::InProcessing);
    }
    Processing::state(OrderState::Finished)
}

/// Casts a player skill: the cooldown is the player's, the cost is paid from
/// the stockpile, and the effect lands on the casting player.
///
/// Refuses silently — a cooldown that has not come round, or a cost the player
/// cannot pay, is a cast that does not happen.
pub fn by_player(
    world: &mut World,
    player: PlayerId,
    skill: SkillId,
    cooldown: u32,
    price: &Price,
    effect: PlayerCastEffect,
) {
    if !world.resource::<PlayerSkills>().ready(player, skill) {
        return;
    }
    if !world
        .resource::<PlayerResources>()
        .can_afford(player, price)
    {
        return;
    }
    resources::charge(world, player, price.clone(), SpendCause::Skill { skill });

    match effect {
        PlayerCastEffect::ApplyBuff(buff) => buffs::apply_player_buff(world, player, buff),
        PlayerCastEffect::RemoveBuff(buff) => buffs::remove_player_buff(world, player, buff),
    }

    player_skills::cast(world, player, skill, cooldown);
}

/// Judges `caster`'s cast of `skill` for `player` as it is ordered, before
/// anything the caster was doing is canceled: the skill's requirements, the
/// caster able to start it, a summon of a bound tunnel end, the cooldown, and
/// the aim where it is whole already — the caster itself, or the picked
/// target of a transient cast or of a caster that may not close (see
/// [`entity_def::may_close`]), which must lie within its reach from where the
/// caster stands. Any other aim is judged as
/// the cast's order runs.
///
/// Panics when `skill` is not cast by an entity.
pub fn judge_issue(
    world: &World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    aiming: Aiming,
) -> Result<(), CastRefusal> {
    let registry = world.resource::<ContentRegistry>();
    let def = registry
        .skill_def(skill)
        .expect("a skill id comes from the registry");
    let (casting, effect) = match &def.caster {
        SkillCaster::Entity {
            casting, effect, ..
        } => (casting, effect),
        SkillCaster::Player { .. } => panic!("judge_issue is asked of a skill an entity casts"),
    };
    if !requirements::met(world, player, Some(caster), &def.requires) {
        return Err(CastRefusal::Unmet);
    }
    startable(world, caster, skill).map_err(CastRefusal::Caster)?;
    match effect {
        EntityCastEffect::Summon { entity_type, .. } => {
            if tunnels::refuses_raising(world, caster, registry.def(*entity_type)) {
                return Err(CastRefusal::Bound);
            }
        }
        EntityCastEffect::ApplyBuff { .. }
        | EntityCastEffect::RemoveBuff { .. }
        | EntityCastEffect::Damage { .. }
        | EntityCastEffect::Heal { .. }
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Relocate { .. } => {}
    }
    if !ready(world, caster, skill) {
        return Err(CastRefusal::CoolingDown);
    }
    let target = match effect.target() {
        EntityCastTarget::Caster => None,
        EntityCastTarget::Position { .. }
        | EntityCastTarget::Standing { .. }
        | EntityCastTarget::Fallen { .. } => match (casting, aiming) {
            (Casting::Transient, Aiming::Picked(target)) => target,
            // A caster that may not close casts from where it stands, as a
            // transient cast does.
            (Casting::Instant | Casting::Delayed { .. }, Aiming::Picked(target)) => {
                match entity_def::may_close(world, caster) {
                    true => return Ok(()),
                    false => target,
                }
            }
            (Casting::Transient | Casting::Instant | Casting::Delayed { .. }, Aiming::Unpicked) => {
                return Ok(());
            }
        },
    };
    let aim = aim(world, player, caster, skill, target).map_err(CastRefusal::Aim)?;
    match reaches_aim(world, caster, skill, aim) {
        true => Ok(()),
        false => Err(CastRefusal::OutOfReach),
    }
}

/// Lands `caster`'s transient cast of `skill` at `target` here and now,
/// without an order: judged, paid and carried out as an instant cast lands.
///
/// Panics when `skill` is not cast transient by an entity, or its aim was not
/// judged first (see [`judge_issue`]).
pub(super) fn transient(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    target: Option<SkillTarget>,
) -> CastLanding {
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a skill id comes from the registry")
        .clone();
    match &def.caster {
        SkillCaster::Entity {
            casting: Casting::Transient,
            ..
        } => {}
        SkillCaster::Entity {
            casting: Casting::Instant | Casting::Delayed { .. },
            ..
        }
        | SkillCaster::Player { .. } => {
            panic!("transient is asked of a skill an entity casts transient")
        }
    }
    let aim = aim(world, player, caster, skill, target)
        .expect("a transient cast's aim is judged before it lands");
    land(world, player, caster, skill, &def, aim, Paid::Nothing)
}

/// Why a cast cannot take what it was aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimRefusal {
    /// The cast takes a target of another shape than it was given: a cell
    /// for an entity, or none at all.
    Unaimed,
    /// The cell lies off the map.
    OffMap,
    /// The entity is not one the caster's owner makes out, or is gone.
    Unseen,
    /// The entity stands on a side the cast does not take.
    WrongSide,
    /// The entity is not a kind the cast takes.
    WrongKind,
    /// The entity cannot take what the cast does to it: a buff modifying a
    /// stat it lacks, or a heal without a health pool.
    Uncarried,
    /// The entity does not meet the requirement the applied buff is held
    /// `While`.
    Unheld,
    /// The cell is where the cast would set bodies down, and the caster's side
    /// does not see it as the cast's landing asks.
    LandingUnseen,
}

/// Why a cast is refused as it is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastRefusal {
    /// The skill's requirements are unmet.
    Unmet,
    /// The caster cannot start the cast: the order gate's refusal.
    Caster(Refusal),
    /// It summons an end of the caster's tunnel, which has its other end
    /// already.
    Bound,
    /// The skill is cooling down.
    CoolingDown,
    /// Its aim refuses it.
    Aim(AimRefusal),
    /// It is cast from where its caster stands, and its aim lies out of reach.
    OutOfReach,
}

/// How much of a cast's aim is chosen when its order is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aiming {
    /// None yet: the player has still to pick a target.
    Unpicked,
    /// Picked: this target, or none for a cast on its caster.
    Picked(Option<SkillTarget>),
}

/// Resolves what `caster`'s cast of `skill` acts on when aimed at `target`, or
/// why it cannot take it — an entity that cannot carry or hold the buff the
/// cast applies included.
///
/// Fog applies to every named aim: what a player cannot see, it cannot name.
/// Panics when `skill` is not cast by an entity.
pub fn aim(
    world: &World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    target: Option<SkillTarget>,
) -> Result<CastAim, AimRefusal> {
    let aim = resolve(world, player, caster, skill, target)?;
    holds(world, skill, aim)?;
    Ok(aim)
}

/// Whether `caster` reaches `aim` with `skill` from where it can get to: any
/// aim, for a cast declaring no reach or a caster that walks into it; for one
/// that may not close (see [`entity_def::may_close`]), or a transient cast,
/// which never walks, an aim within the reach of every cell it stands on, as
/// the cast order measures it.
///
/// Panics when `skill` is not cast by an entity.
pub fn reaches_aim(world: &World, caster: Entity, skill: SkillId, aim: CastAim) -> bool {
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a skill id comes from the registry");
    let Some(range) = reach_of(world, caster, def) else {
        return true;
    };
    let walks = match &def.caster {
        SkillCaster::Entity { casting, .. } => match casting {
            Casting::Transient => false,
            Casting::Instant | Casting::Delayed { .. } => entity_def::may_close(world, caster),
        },
        SkillCaster::Player { .. } => panic!("reaches_aim is asked of a skill an entity casts"),
    };
    match walks {
        true => true,
        false => reach::in_reach(world, caster, aimed_standing_rect(world, aim), range),
    }
}

/// Where a resolved entity cast lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastAim {
    /// On an entity.
    Entity(Entity),
    /// On a cell.
    Cell(CellPos),
    /// On remains, which the cast spends.
    Remains(Entity),
}

/// What a judged cast is about to do.
enum CastPlan {
    /// Act on what the cast was aimed at.
    Act,
    /// Set summoned units down, one on each cell.
    Summon {
        /// The type summoned.
        entity_type: EntityTypeId,
        /// The anchor each unit lands on, and what it covers there.
        cells: Vec<(CellPos, Cover)>,
        /// Whether they stand at once or rise.
        arrives: SummonArrival,
    },
    /// Move each body to stand anchored at its cell.
    Relocate { moves: Vec<(Entity, CellPos)> },
}

/// What came of a cast's start.
enum CastStart {
    /// It started, having paid as this says.
    Started(Paid),
    /// It was refused: requirements its caster no longer meets, nothing it
    /// would do at its aim, or a cost it could not pay. Nothing was paid, and
    /// its order ends.
    Refused,
}

/// What came of a cast at its point.
pub(super) enum CastLanding {
    /// It landed: judged, paid and carried out.
    Landed,
    /// It was refused: for a cast that had paid nothing, requirements its
    /// caster no longer meets or a buff its aim would not hold; no ground for
    /// what it would set down, nothing to move, or a cost it could not pay.
    /// What it paid at its start was given back as its terms say.
    Refused,
}

/// How long the cast holds `caster`, in ticks: when it lands, and when the
/// caster is free of it.
///
/// An instant cast lands on the first tick and frees the caster with it. A
/// worked one lands on the tick its point names — never zero, which
/// registration refuses of a constant and the stat's own floor refuses of a
/// stat. Neither number is bent to the other: a period a stat has pulled under
/// its own point is a cast that lands and frees the caster on the same tick.
fn casting(world: &World, caster: Entity, def: &SkillDef) -> (u32, u32) {
    match &def.caster {
        SkillCaster::Entity { casting, .. } => match casting {
            Casting::Transient => unreachable!("a transient cast never becomes an order"),
            Casting::Instant => (1, 1),
            Casting::Delayed { point, period, .. } => (
                entity_def::quantity_ticks(world, caster, *point),
                entity_def::quantity_ticks(world, caster, *period),
            ),
        },
        SkillCaster::Player { .. } => unreachable!("an entity cast carries an entity arm"),
    }
}

/// How far `caster` casts `def` from, in cells, or `None` for a cast made
/// where it stands. A player cast has no position, so it closes on nothing.
///
/// A reach read from a stat is read off the caster's effective stats, so a
/// modifier that lengthens it reaches a caster already walking.
fn reach_of(world: &World, caster: Entity, def: &SkillDef) -> Option<u32> {
    match &def.caster {
        SkillCaster::Entity { effect, .. } => match effect.target() {
            EntityCastTarget::Position { reach }
            | EntityCastTarget::Standing { reach, .. }
            | EntityCastTarget::Fallen { reach, .. } => match reach {
                Reach::Wherever => None,
                Reach::Within(reach) => Some(entity_def::quantity_cells(world, caster, reach)),
            },
            EntityCastTarget::Caster => None,
        },
        SkillCaster::Player { .. } => None,
    }
}

/// Starts the casting of `skill` at `aim`: refused when the caster no longer
/// meets the skill's requirements; a cast that pays at its start is judged —
/// what it would do there, its cost — paid, and put on cooldown here, or
/// refused; one that pays when it lands owes nothing yet.
fn start(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    def: &SkillDef,
    aim: CastAim,
) -> CastStart {
    let SkillCaster::Entity {
        costs,
        casting,
        effect,
    } = &def.caster
    else {
        unreachable!("a type declares entity casts only, so only one is cast from one")
    };
    if !requirements::met(world, player, Some(caster), &def.requires) {
        return CastStart::Refused;
    }
    let refund = match casting {
        Casting::Transient => unreachable!("a transient cast lands without starting"),
        Casting::Instant
        | Casting::Delayed {
            pays: PaymentPoint::AtLanding,
            ..
        } => return CastStart::Started(Paid::Nothing),
        Casting::Delayed {
            pays: PaymentPoint::AtStart(refund),
            ..
        } => *refund,
    };
    if judge(world, player, caster, effect, aim).is_none()
        || !cost::can_pay(world, caster, player, costs)
    {
        return CastStart::Refused;
    }
    cost::pay(world, caster, player, costs, SpendCause::Skill { skill });
    entity_skills::start_cooldown(world, caster, skill, def.cooldown);
    CastStart::Started(Paid::AtStart { player, refund })
}

/// Lands `skill` from `caster` at `aim`, here and now: a cast that has paid
/// nothing yet is refused when the caster no longer meets the skill's
/// requirements or the buff it applies would not hold on its aim, and
/// otherwise judged and paid as it lands; one that paid at its start has its
/// effect judged again, and a refused landing gives back what its terms say.
fn land(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    def: &SkillDef,
    aim: CastAim,
    paid: Paid,
) -> CastLanding {
    let SkillCaster::Entity { costs, effect, .. } = &def.caster else {
        unreachable!("a type declares entity casts only, so only one is cast from one")
    };
    match paid {
        Paid::Nothing => {
            if !requirements::met(world, player, Some(caster), &def.requires)
                || holds(world, skill, aim).is_err()
            {
                return CastLanding::Refused;
            }
        }
        Paid::AtStart { .. } => {}
    }
    let Some(plan) = judge(world, player, caster, effect, aim) else {
        give_back(world, caster, skill, paid);
        return CastLanding::Refused;
    };
    match paid {
        Paid::Nothing => {
            if !cost::can_pay(world, caster, player, costs) {
                return CastLanding::Refused;
            }
            cost::pay(world, caster, player, costs, SpendCause::Skill { skill });
            let landed_on = carry_out(world, player, caster, skill, aim, effect, plan);
            entity_skills::start_cooldown(world, caster, skill, def.cooldown);
            entity_skills::announce_cast(world, caster, landed_on, skill);
        }
        Paid::AtStart { .. } => {
            let landed_on = carry_out(world, player, caster, skill, aim, effect, plan);
            entity_skills::announce_cast(world, caster, landed_on, skill);
        }
    }
    CastLanding::Landed
}

/// Carries out a judged and paid cast of `skill` at `aim`, returning what the
/// cast is announced against.
fn carry_out(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    aim: CastAim,
    effect: &EntityCastEffect,
    plan: CastPlan,
) -> SkillTarget {
    buffs::interrupt_entity_buffs(world, caster, Interruption::Cast);

    let caster_id = entity_def::simulation_id(world, caster);
    // The cast is announced against what it landed on, and a body it landed on
    // is spent by landing there.
    let (landed_on, spent) = match aim {
        CastAim::Entity(target) => (
            SkillTarget::Entity(entity_def::simulation_id(world, target)),
            None,
        ),
        // A body is spent by the landing, so the cast names the ground it lay
        // on: naming the body would name something already gone.
        CastAim::Remains(remains) => {
            let (spent, lay) = spend_remains(world, caster_id, remains);
            (SkillTarget::Position(lay), Some(spent))
        }
        CastAim::Cell(cell) => (SkillTarget::Position(FixedUVec2::from(cell)), None),
    };
    perform(world, player, caster, skill, aim, effect, plan, spent);
    landed_on
}

/// Ends a cast cut short before it landed, giving back what its terms say.
fn cut_short(world: &mut World, caster: Entity, skill: SkillId, paid: Paid) -> Processing {
    give_back(world, caster, skill, paid);
    Processing::state(OrderState::Finished)
}

/// Gives back what a cast cut short before it landed paid at its start, when
/// its terms say so: every cost — resources to whoever paid them — and its
/// cooldown reset. A caster that is dying gets nothing back.
fn give_back(world: &mut World, caster: Entity, skill: SkillId, paid: Paid) {
    let player = match paid {
        Paid::Nothing
        | Paid::AtStart {
            refund: Refund::None,
            ..
        } => return,
        Paid::AtStart {
            player,
            refund: Refund::Full,
        } => player,
    };
    if entity_def::dying(world, caster) {
        return;
    }
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a declared skill id comes from the registry");
    let SkillCaster::Entity { costs, .. } = &def.caster else {
        unreachable!("a type declares entity casts only, so only one is cast from one")
    };
    let costs = costs.clone();
    cost::refund(world, caster, player, &costs, SpendCause::Skill { skill });
    entity_skills::reset_cooldown(world, caster, skill);
}

/// What `caster`'s cast of `skill` acts on when aimed at `target`, or why it
/// cannot take it, whether the buff it applies would hold there aside.
fn resolve(
    world: &World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    target: Option<SkillTarget>,
) -> Result<CastAim, AimRefusal> {
    let effect = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a skill id comes from the registry")
        .entity_effect()
        .expect("resolve is asked of a skill an entity casts");
    match effect.target() {
        EntityCastTarget::Caster => {
            takes(world, caster, effect)?;
            Ok(CastAim::Entity(caster))
        }
        EntityCastTarget::Position { .. } => {
            let Some(SkillTarget::Position(position)) = target else {
                return Err(AimRefusal::Unaimed);
            };
            let cell = CellPos::from(position);
            if !world.resource::<Map>().contains(cell) {
                return Err(AimRefusal::OffMap);
            }
            match lands_at_aim(effect) {
                Some(landing)
                    if !landing::sees(
                        world,
                        player,
                        landing,
                        CellRect::new(cell, CellSize::ONE),
                    ) =>
                {
                    Err(AimRefusal::LandingUnseen)
                }
                Some(_) | None => Ok(CastAim::Cell(cell)),
            }
        }
        EntityCastTarget::Fallen { kinds, .. } => {
            let Some(SkillTarget::Entity(id)) = target else {
                return Err(AimRefusal::Unaimed);
            };
            // Anyone's body serves: what fell there stopped belonging to
            // anybody when it fell.
            let remains =
                visibility::remains_interactable_to(world, player, id).ok_or(AimRefusal::Unseen)?;
            if !names(world, remains, kinds) {
                return Err(AimRefusal::WrongKind);
            }
            Ok(CastAim::Remains(remains))
        }
        EntityCastTarget::Standing {
            whom: Whom { side, kinds },
            ..
        } => {
            let Some(SkillTarget::Entity(id)) = target else {
                return Err(AimRefusal::Unaimed);
            };
            let target =
                visibility::interactable_to(world, player, id).ok_or(AimRefusal::Unseen)?;
            let session = world.resource::<GameSession>();
            let caster_owner = entity_def::owner(world, caster);
            let target_owner = entity_def::owner(world, target);
            if !owner::admits(session, *side, caster_owner, target_owner) {
                return Err(AimRefusal::WrongSide);
            }
            if !names(world, target, kinds) {
                return Err(AimRefusal::WrongKind);
            }
            takes(world, target, effect)?;
            Ok(CastAim::Entity(target))
        }
    }
}

/// Whether `entity` can take what `effect` does to it: a buff it carries, a
/// heal when it has health, or any other effect.
fn takes(world: &World, entity: Entity, effect: &EntityCastEffect) -> Result<(), AimRefusal> {
    match effect {
        EntityCastEffect::ApplyBuff { buff, .. } => {
            if entity_def::can_carry(world, entity, *buff) {
                Ok(())
            } else {
                Err(AimRefusal::Uncarried)
            }
        }
        EntityCastEffect::Heal { .. } => {
            if entity_def::has_pool(world, entity, PoolId::HEALTH) {
                Ok(())
            } else {
                Err(AimRefusal::Uncarried)
            }
        }
        EntityCastEffect::RemoveBuff { .. }
        | EntityCastEffect::Damage { .. }
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Summon { .. }
        | EntityCastEffect::Relocate { .. } => Ok(()),
    }
}

/// Whether the buff `skill` applies would hold on `aim`: a buff held `While`
/// a requirement holds on a recipient that meets it, read for the recipient's
/// owner. Any other effect holds.
fn holds(world: &World, skill: SkillId, aim: CastAim) -> Result<(), AimRefusal> {
    let registry = world.resource::<ContentRegistry>();
    let Some(buff) = registry
        .skill_def(skill)
        .expect("a skill id comes from the registry")
        .entity_effect()
        .expect("holds is asked of a skill an entity casts")
        .applied_buff()
    else {
        return Ok(());
    };
    let recipient = match aim {
        CastAim::Entity(recipient) => recipient,
        CastAim::Cell(_) | CastAim::Remains(_) => return Ok(()),
    };
    let Some(requirement) = registry.entity_buff_def(buff).lasts_while() else {
        return Ok(());
    };
    let owner = entity_def::owner(world, recipient);
    match requirements::met_by(world, owner, recipient, requirement) {
        true => Ok(()),
        false => Err(AimRefusal::Unheld),
    }
}

/// The landing a cast sets bodies down by at the cell it is aimed at, if it
/// sets any down there: a relocation from the caster to the aim.
fn lands_at_aim(effect: &EntityCastEffect) -> Option<Landing> {
    match effect {
        EntityCastEffect::Relocate { journey, landing } => match journey.as_ref() {
            Journey::FromCaster {
                to: FromCasterTo::Aim(_),
                ..
            } => Some(*landing),
            Journey::FromCaster {
                to: FromCasterTo::Home(_),
                ..
            }
            | Journey::FromAim { .. }
            | Journey::Swap { .. } => None,
        },
        EntityCastEffect::ApplyBuff { .. }
        | EntityCastEffect::RemoveBuff { .. }
        | EntityCastEffect::Damage { .. }
        | EntityCastEffect::Heal { .. }
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Summon { .. } => None,
    }
}

/// Whether `kinds` names what `entity` is.
fn names(world: &World, entity: Entity, kinds: &Kinds) -> bool {
    kinds.admits(entity_def::of(world, entity))
}

/// Settles what the cast will do, or `None` when it cannot be done at all.
///
/// A summon and a relocation can fail here. The units a summon calls up need
/// ground to stand on and headroom in the owner's supply, and a summon that
/// cannot set every one of them down does nothing rather than half of it. A
/// relocation needs somewhere to go, something to move, and ground for what it
/// moves there.
fn judge(
    world: &World,
    player: PlayerId,
    caster: Entity,
    effect: &EntityCastEffect,
    aim: CastAim,
) -> Option<CastPlan> {
    match effect {
        EntityCastEffect::Summon {
            entity_type,
            count,
            placement,
            landing,
            within,
            arrives,
            ..
        } => summoning(
            world,
            player,
            caster,
            *entity_type,
            *count,
            *placement,
            *landing,
            within,
            *arrives,
            aim,
        ),
        EntityCastEffect::Relocate { journey, landing } => {
            relocation(world, player, caster, journey, *landing, aim)
        }
        EntityCastEffect::ApplyBuff { .. }
        | EntityCastEffect::RemoveBuff { .. }
        | EntityCastEffect::Damage { .. }
        | EntityCastEffect::Heal { .. }
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. } => Some(CastPlan::Act),
    }
}

/// Settles where a summon of `count` units of `entity_type` sets each one
/// down, or `None` when it cannot set every one down: no headroom in the
/// owner's supply, or not ground enough around the aim.
///
/// The units stand where they may — on the aimed cell for an exact
/// placement, or for a type raised over a source on the source under it;
/// nearest the aim for a nearby one; over the ground of remains aimed at,
/// which the cast spends first: the same ground, placement
/// rules, landing and order a relocation lands by, inside the summon's
/// `within` field rules; a type raised over a source lands only over a
/// source of its type, which it covers.
#[allow(clippy::too_many_arguments)]
fn summoning(
    world: &World,
    player: PlayerId,
    caster: Entity,
    entity_type: EntityTypeId,
    count: Quantity,
    placement: SummonPlacement,
    landing: Landing,
    within: &[FieldPlacement],
    arrives: SummonArrival,
    aim: CastAim,
) -> Option<CastPlan> {
    let count = entity_def::quantity_count(world, caster, count);
    let def = world.resource::<ContentRegistry>().def(entity_type);
    if !supply::allows_all(world, player, def, count as usize) {
        return None;
    }
    // A bound tunnel end has its other end already.
    if tunnels::refuses_raising(world, caster, def) {
        return None;
    }
    let around = aimed_standing_rect(world, aim);
    // Remains aimed at are spent before the bodies land, so their ground is
    // free for them.
    let (lifted, ignored) = match aim {
        CastAim::Remains(remains) => (
            entity_def::grid_mark(world, remains).into_iter().collect(),
            vec![remains],
        ),
        CastAim::Entity(_) | CastAim::Cell(_) => (Vec::new(), Vec::new()),
    };
    let mut landings = Landings::new(player, landing, within, &lifted, &ignored);
    let from = aimed_source(world, aim);
    // An exact body raised over a source anchors on the source under the
    // aimed cell; any other on the aimed cell itself.
    let bound = match (placement, def.overbuilds.as_deref()) {
        (SummonPlacement::Exact, Some(over)) => {
            let source = overbuild::source_covering(world, over, around.origin)?;
            Bound::At(entity_def::footprint_rect(world, source).origin)
        }
        (SummonPlacement::Exact, None) => Bound::At(around.origin),
        (SummonPlacement::Nearby, _) => Bound::Radius,
    };
    let lander = Lander {
        def,
        owner: Some(player),
        from,
        bound,
        around,
        toward: around,
    };
    // A type raised over a source lands on one of that type, which it covers.
    let cells = (0..count)
        .map(|_| match def.overbuilds.as_deref() {
            Some(over) => landing::land_over(world, &mut landings, over, lander)
                .map(|(cell, source)| (cell, Cover::Source(source))),
            None => landing::land(world, &mut landings, lander).map(|cell| (cell, Cover::Nothing)),
        })
        .collect::<Option<Vec<(CellPos, Cover)>>>()?;
    Some(CastPlan::Summon {
        entity_type,
        cells,
        arrives,
    })
}

/// Settles where a relocation on `journey` sets each body down, or `None` when
/// it cannot be done: no home to go to, the caster or the aimed body not one
/// that can be moved or fitting nowhere, a gathering that refuses to leave
/// any of its bodies behind with some fitting nowhere, or nobody to move.
///
/// The caster is set down first when it goes, then gathered bodies nearest
/// their gathering spot first, ties by id; each lands nearest where it goes,
/// leaning toward where it stood, and the cells every body moved holds now do
/// not stand in any of their ways.
fn relocation(
    world: &World,
    player: PlayerId,
    caster: Entity,
    journey: &Journey,
    landing: Landing,
    aim: CastAim,
) -> Option<CastPlan> {
    let caster_rect = entity_def::standing_rect(world, caster);
    let aim_rect = aimed_standing_rect(world, aim);
    let aimed = match aim {
        CastAim::Entity(entity) => Some(entity),
        CastAim::Cell(_) | CastAim::Remains(_) => None,
    };
    // Aimed at its own caster, a journey that goes to or from the aim has
    // nothing to take anywhere; one home from the caster is aimed there.
    let goes_by_aim = match journey {
        Journey::FromCaster {
            to: FromCasterTo::Home(_),
            ..
        } => false,
        Journey::FromCaster {
            to: FromCasterTo::Aim(_),
            ..
        }
        | Journey::FromAim { .. }
        | Journey::Swap { .. } => true,
    };
    if goes_by_aim && aimed == Some(caster) {
        return None;
    }
    // The caster, an aimed body or a swap's pair, each with where it goes;
    // and the bodies gathered, where they all go, and what a shortfall does.
    let (singles, gathered) = match journey {
        Journey::FromCaster { party, to } => {
            // The body a journey goes to is never among those it moves.
            let (destination, goes_to) = match to {
                FromCasterTo::Aim(_) => ((aim_rect, aimed_source(world, aim)), aimed),
                FromCasterTo::Home(kinds) => {
                    let home = home(world, player, kinds, caster_rect, &[caster])?;
                    (
                        (
                            entity_def::standing_rect(world, home),
                            ComesFrom::of(world, home),
                        ),
                        Some(home),
                    )
                }
            };
            let not_gathered: Vec<Entity> = [Some(caster), goes_to].into_iter().flatten().collect();
            let around_caster = |gathering: &Gathering| {
                let bodies = gather(
                    world,
                    player,
                    caster,
                    gathering,
                    entity_def::standing_rect(world, caster),
                    &not_gathered,
                );
                Some((bodies, destination, gathering.shortfall))
            };
            match party {
                CasterParty::Caster => (vec![(caster, destination)], None),
                CasterParty::CasterAndAround(gathering) => {
                    (vec![(caster, destination)], around_caster(gathering))
                }
                CasterParty::Around(gathering) => (Vec::new(), around_caster(gathering)),
            }
        }
        Journey::FromAim { party, to } => {
            let target = || aimed.expect("a party at the aim of a target is aimed at an entity");
            let not_home: Vec<Entity> = match party {
                AimParty::Target { .. } => vec![target()],
                AimParty::Around { .. } => Vec::new(),
            };
            let (destination, goes_to) = match to {
                FromAimTo::Caster => ((caster_rect, ComesFrom::of(world, caster)), caster),
                FromAimTo::Home(kinds) => {
                    let home = home(world, player, kinds, aim_rect, &not_home)?;
                    (
                        (
                            entity_def::standing_rect(world, home),
                            ComesFrom::of(world, home),
                        ),
                        home,
                    )
                }
            };
            match party {
                AimParty::Target { .. } => (vec![(target(), destination)], None),
                AimParty::Around { gathering, .. } => {
                    let bodies = gather(
                        world,
                        player,
                        caster,
                        gathering,
                        aimed_standing_rect(world, aim),
                        &[goes_to],
                    );
                    (Vec::new(), Some((bodies, destination, gathering.shortfall)))
                }
            }
        }
        // Each lands nearest where the other stood.
        Journey::Swap { .. } => {
            let target = aimed.expect("a swap is aimed at an entity");
            let target_rect = entity_def::standing_rect(world, target);
            (
                vec![
                    (caster, (target_rect, ComesFrom::of(world, target))),
                    (target, (caster_rect, ComesFrom::of(world, caster))),
                ],
                None,
            )
        }
    };
    // The caster and an aimed body either go or the cast does not.
    if singles
        .iter()
        .any(|&(entity, _)| !entity_def::relocatable(world, entity))
    {
        return None;
    }

    let bodies: Vec<Entity> = singles
        .iter()
        .map(|&(entity, _)| entity)
        .chain(
            gathered
                .iter()
                .flat_map(|(gathered, ..)| gathered.iter().copied()),
        )
        .collect();
    let lifted: Vec<_> = bodies
        .iter()
        .filter_map(|&entity| entity_def::grid_mark(world, entity))
        .collect();
    let mut landings = Landings::new(player, landing, &[], &lifted, &bodies);
    // Each lands nearest where it goes, leaning toward the ground it stood on.
    let mut land = |entity: Entity, (around, from): (CellRect, ComesFrom)| {
        let lander = Lander {
            def: entity_def::of(world, entity),
            owner: entity_def::owner(world, entity),
            from,
            bound: Bound::Radius,
            around,
            toward: entity_def::footprint_rect(world, entity),
        };
        landing::land(world, &mut landings, lander)
    };
    let mut moves: Vec<(Entity, CellPos)> = Vec::new();
    for &(entity, around) in &singles {
        moves.push((entity, land(entity, around)?));
    }
    if let Some((gathered, around, shortfall)) = &gathered {
        for &entity in gathered {
            match (land(entity, *around), shortfall) {
                (Some(cell), _) => moves.push((entity, cell)),
                (None, Shortfall::Refuse) => return None,
                (None, Shortfall::Leave) => {}
            }
        }
    }
    if moves.is_empty() {
        return None;
    }
    Some(CastPlan::Relocate { moves })
}

/// The bodies `gathering` takes around the cells `center`, nearest first,
/// ties by id, up to its count: each one that can be moved, standing within
/// its radius by map distance, on its side judged from `player`, of its kinds,
/// and made out by `player` where its sight asks. None of `not_gathered` is
/// taken.
fn gather(
    world: &World,
    player: PlayerId,
    caster: Entity,
    gathering: &Gathering,
    center: CellRect,
    not_gathered: &[Entity],
) -> Vec<Entity> {
    let radius = entity_def::quantity_cells(world, caster, gathering.radius);
    let projection = world.resource::<Map>().projection();
    let session = world.resource::<GameSession>();
    let mut found: Vec<(u32, SimulationId, Entity)> = world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter_map(|(id, entity)| {
            if not_gathered.contains(&entity) || !entity_def::relocatable(world, entity) {
                return None;
            }
            let standing = entity_def::standing_rect(world, entity);
            if !projection.in_range_for_rects(standing, center, radius) {
                return None;
            }
            let owner = entity_def::owner(world, entity);
            let admitted = owner::admits(session, gathering.side, Some(player), owner)
                && names(world, entity, &gathering.kinds)
                && match gathering.sight {
                    GatherSight::Blind => true,
                    GatherSight::Seen => {
                        visibility::sees(world, player, entity, Senses::SeatDeclared)
                    }
                };
            admitted.then(|| (projection.distance_for_rects(standing, center), id, entity))
        })
        .collect();
    found.sort_unstable_by_key(|&(distance, id, _)| (distance, id));
    let count = match gathering.count {
        Count::All => found.len(),
        Count::AtMost(count) => entity_def::quantity_count(world, caster, count) as usize,
    };
    found
        .into_iter()
        .take(count)
        .map(|(_, _, entity)| entity)
        .collect()
}

/// The home a journey goes to: `player`'s building of `kinds` standing on the
/// map and finished, nearest `spot` by map distance to the cells it stands on,
/// ties by id, none of `not_home`. `None` when there is none at all.
fn home(
    world: &World,
    player: PlayerId,
    kinds: &Kinds,
    spot: CellRect,
    not_home: &[Entity],
) -> Option<Entity> {
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter(|&(_, entity)| {
            !not_home.contains(&entity)
                && entity_def::owner(world, entity) == Some(player)
                && entity_def::stands(world, entity)
                && !entity_def::rising(world, entity)
                && names(world, entity, kinds)
        })
        .min_by_key(|&(id, entity)| (reach::distance_to(world, spot, entity), id))
        .map(|(_, entity)| entity)
}

/// The cells a resolved `aim` is reached at: the aimed cell, or every cell
/// the aimed body stands on.
fn aimed_standing_rect(world: &World, aim: CastAim) -> CellRect {
    match aim {
        CastAim::Cell(cell) => CellRect::new(cell, CellSize::ONE),
        CastAim::Entity(target) | CastAim::Remains(target) => {
            entity_def::standing_rect(world, target)
        }
    }
}

/// Where a body set down at a resolved `aim` comes from: the aimed cell, or
/// out of the aimed body's footprint.
fn aimed_source(world: &World, aim: CastAim) -> ComesFrom {
    match aim {
        CastAim::Cell(cell) => ComesFrom::Point(cell),
        CastAim::Entity(target) | CastAim::Remains(target) => ComesFrom::of(world, target),
    }
}

/// Takes `remains` off the map, announcing what spent them and where they lay.
///
/// Returns their id and that ground: the id for the summon that comes out of
/// them, the ground for the cast that names where it landed. Both are read
/// here because both are gone the moment the body is.
fn spend_remains(
    world: &mut World,
    caster: SimulationId,
    remains: Entity,
) -> (SimulationId, FixedUVec2) {
    let id = entity_def::simulation_id(world, remains);
    let position = entity_def::position(world, remains);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::RemainsSpent {
            remains: id,
            position,
            by: caster,
        });
    spawn::despawn_remains(world, remains);
    (id, position)
}

/// Carries out a judged cast of `skill` at `aim`.
#[allow(clippy::too_many_arguments)]
fn perform(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    aim: CastAim,
    effect: &EntityCastEffect,
    plan: CastPlan,
    from_remains: Option<SimulationId>,
) {
    let caster_id = entity_def::simulation_id(world, caster);
    match plan {
        CastPlan::Summon {
            entity_type,
            cells,
            arrives,
        } => {
            let type_name = world
                .resource::<ContentRegistry>()
                .def(entity_type)
                .name
                .clone();
            // What comes out of a body remembers the body it came from; what is
            // called up out of nothing names only its caster.
            for (cell, cover) in cells {
                let position = FixedUVec2::from(cell);
                // What a cast calls up is as new as anything trained: its
                // field sources start at their initial reach.
                let reach = FieldReach::Initial;
                // A rising one rises on its own, for its caster.
                let arrival = match arrives {
                    SummonArrival::Standing => Arrival::Standing { starts: &[] },
                    SummonArrival::Rising => Arrival::Rising {
                        attendance: RiseAttendance::Unattended { founder: caster_id },
                        origin: RiseOrigin::Summoned { skill },
                    },
                };
                // A source the summon is raised over comes off the grid so
                // the summon takes its cells, and is covered once it stands.
                overbuild::lift(world, cover);
                let summoned = match from_remains {
                    Some(from) => spawn::spawn_raised(
                        world,
                        &type_name,
                        position,
                        Some(player),
                        reach,
                        arrival,
                        caster_id,
                        from,
                    ),
                    None => spawn::spawn_summoned(
                        world,
                        &type_name,
                        position,
                        Some(player),
                        reach,
                        arrival,
                        caster_id,
                    ),
                };
                overbuild::settle(world, cover, summoned.map(|(unit, _)| unit));
            }
        }
        CastPlan::Relocate { moves } => {
            // Every body moved is given somewhere new to be, as by a new
            // command — but the caster of a cast made through an order, which
            // is inside its own cast and carries on with what it queued
            // behind it, from where the cast set it down.
            let moved = moves.iter().map(|&(entity, _)| entity);
            let dropped: Vec<Entity> = match world
                .resource::<ContentRegistry>()
                .skill_def(skill)
                .expect("a skill id comes from the registry")
                .caster
            {
                SkillCaster::Entity {
                    casting: Casting::Transient,
                    ..
                } => moved.collect(),
                SkillCaster::Entity {
                    casting: Casting::Instant | Casting::Delayed { .. },
                    ..
                } => moved.filter(|&entity| entity != caster).collect(),
                SkillCaster::Player { .. } => {
                    unreachable!("a type declares entity casts only, so only one is cast from one")
                }
            };
            for entity in dropped {
                orders::drop_all(world, entity, CancelPolicy::Soft);
            }
            spawn::relocate(world, &moves);
        }
        CastPlan::Act => apply_effect(world, player, caster, aim, effect),
    }
}

/// Applies an effect that acts where it was aimed.
fn apply_effect(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    aim: CastAim,
    effect: &EntityCastEffect,
) {
    // The two effects that act on a patch of ground rather than on a thing:
    // both read the aim as a cell, whether it was aimed there or at something
    // standing there.
    match effect {
        &EntityCastEffect::Field {
            field,
            radius,
            action,
            ..
        } => {
            let center = aimed_cell(world, aim);
            let radius = entity_def::quantity_cells(world, caster, radius);
            fields::apply_action(world, player, field, center, radius, action);
            return;
        }
        &EntityCastEffect::Watch {
            radius,
            duration,
            detection,
            ..
        } => {
            let center = aimed_cell(world, aim);
            let radius = entity_def::quantity_cells(world, caster, radius);
            let duration = entity_def::quantity_ticks(world, caster, duration);
            let caster_id = entity_def::simulation_id(world, caster);
            watches::open(
                world, player, caster_id, center, radius, duration, detection,
            );
            return;
        }
        EntityCastEffect::ApplyBuff { .. }
        | EntityCastEffect::RemoveBuff { .. }
        | EntityCastEffect::Damage { .. }
        | EntityCastEffect::Heal { .. }
        | EntityCastEffect::Summon { .. }
        | EntityCastEffect::Relocate { .. } => {}
    }
    let target = match aim {
        CastAim::Entity(target) => target,
        CastAim::Cell(_) | CastAim::Remains(_) => {
            unreachable!("a buff, damage or heal is aimed at a recipient, never a cell or remains")
        }
    };
    match effect {
        &EntityCastEffect::ApplyBuff { buff: id, .. } => {
            match buffs::apply_entity_buff(world, target, id) {
                Bearing::Borne => {}
                Bearing::Uncarried => {
                    unreachable!("the aim admits only a target that can carry the buff")
                }
            }
        }
        &EntityCastEffect::RemoveBuff { buff: id, .. } => {
            // A target that does not bear the buff loses nothing.
            if entity_def::bears(world, target, id) {
                buffs::remove_entity_buff(world, target, id);
            }
        }
        &EntityCastEffect::Damage { amount, .. } => {
            // Skill damage bypasses armor, like an ability rather than a
            // weapon — and denies nothing: what it kills leaves whatever its
            // type leaves.
            let caster_id = entity_def::simulation_id(world, caster);
            damage::apply(world, caster_id, target, amount, Slain::Remains);
        }
        &EntityCastEffect::Heal { amount, .. } => {
            pools::restore(world, target, PoolId::HEALTH, amount)
        }
        EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Summon { .. } => {
            unreachable!("handled above")
        }
        EntityCastEffect::Relocate { .. } => {
            unreachable!("a relocation is carried out by its plan")
        }
    }
}

/// The cell a cast acts on: the one it was aimed at, or the one its target
/// stands in.
fn aimed_cell(world: &World, aim: CastAim) -> CellPos {
    match aim {
        CastAim::Cell(cell) => cell,
        CastAim::Entity(target) | CastAim::Remains(target) => {
            CellPos::from(entity_def::position(world, target))
        }
    }
}

/// The footprint the caster closes on for a resolved `aim`.
fn aimed_footprint(world: &World, aim: CastAim) -> (FixedUVec2, CellSize) {
    match aim {
        CastAim::Entity(target) | CastAim::Remains(target) => entity_def::footprint(world, target),
        CastAim::Cell(cell) => (FixedUVec2::from(cell), CellSize::ONE),
    }
}

/// Whether `entity` may start casting `skill`: it operates, its type declares
/// the skill, and the requirements its owner must meet are met.
fn startable(world: &World, entity: Entity, skill: SkillId) -> Result<(), Refusal> {
    orders::requires_operating(world, entity)?;
    if !entity_def::of(world, entity).skills.contains(&skill) {
        return Err(Refusal::Incapable);
    }
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a declared skill id comes from the registry");
    let owner = entity_def::owner(world, entity);
    match owner {
        Some(player) if requirements::met(world, player, Some(entity), &def.requires) => Ok(()),
        Some(_) => Err(Refusal::Incapable),
        // Nobody's entity answers to nobody's research, so a cast it was
        // somehow given is one it cannot start.
        None => Err(Refusal::Incapable),
    }
}

/// Whether `caster`'s `skill` has cooled down.
fn ready(world: &World, caster: Entity, skill: SkillId) -> bool {
    world
        .entity(caster)
        .get::<SkillsComponent>()
        .is_some_and(|skills| skills.ready(skill))
}
