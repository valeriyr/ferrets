//! Casting: the Cast order an entity works through, and the casts that belong
//! to a player rather than to anything on the map.
//!
//! An entity's cast is an order like any other — queued, canceled and
//! replaced like a walk — which walks the caster into the skill's reach first
//! when it declares one, and lands where it stands when it does not. A
//! player's cast has no caster to send anywhere, so it happens where the
//! command arrives.
//!
//! The order half is called by [`super::orders`] as part of the shared order
//! lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    attack::Slain,
    entity_buffs::Interruption,
    entity_type_def::EntityTypeId,
    kinds::Kinds,
    pool_def::PoolId,
    price::Price,
    registry::ContentRegistry,
    skills::{
        Casting, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach, SkillCaster,
        SkillDef, SkillId,
    },
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
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
        cast::{CastComponent, CastStage},
        dying::DyingComponent,
        entity_skills::{self, SkillsComponent},
        order_queue::{CancelPolicy, OrderState},
        owner, pools,
    },
    entity_def,
    events::{EventRecord, SimulationEvent, SpendCause},
    game_loop::fields,
    map::Map,
    order::Order,
    player_skills::{self, PlayerSkills},
    requirements,
    resources::{self, PlayerResources},
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
    supply, visibility, watches,
};

/// Whether `entity` may start this Cast: it operates, its type declares the
/// skill, and the requirements its owner must meet are met.
///
/// Whether the aim is still there and whether the cost can be paid are
/// settled at the cast itself, a walk later — as they are for an attack. The
/// cooldown is asked at the order's issue instead, so a skill pressed early
/// costs the caster nothing it was already doing.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let (skill, _) = order.cast_params().expect("Cast order must have a skill");
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

/// Called once when a Cast order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    world.entity_mut(entity).insert(CastComponent::default());
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
/// The cast simply stops. Called off before its point, nothing has been paid
/// or spent; called off after it, the cast has already landed and what it
/// took is gone — the caster is only cut free of the rest of its own cast.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> Processing {
    world.entity_mut(entity).remove::<CastComponent>();
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
/// then work at the cast: it lands on the tick its point names and the caster
/// stands through the rest of its period before the order finishes. The order
/// finishes without casting when the aim is gone — a body raised by someone
/// else, a target that died — or cannot be reached.
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

    // Closing on the aim is the work before the cast lands; what follows the
    // landing is the caster standing through the rest of its own cast, and the
    // aim it spent may well be gone by then. A skill that declares no reach
    // lands from wherever the caster stands, so there is nothing to close on
    // and nothing to turn toward.
    if state.stage == CastStage::Working
        && let Some(range) = reach_of(world, entity, &def)
    {
        let Some(goal) = goal(world, player, entity, skill, target) else {
            return Processing::state(OrderState::Finished);
        };
        let projection = world.resource::<Map>().projection();
        let (own_position, own_size) = entity_def::footprint(world, entity);
        match chase::advance(
            &mut state.last_chase,
            projection,
            own_position,
            own_size,
            goal.0,
            goal.1,
            range,
        ) {
            Destination::OutOfReach => return Processing::state(OrderState::Finished),
            Destination::Walk(move_order) => {
                // The cast starts over when the caster has to walk again, as a
                // swing does: there is no half-worked cast to carry along.
                state.phase = 0;
                world.entity_mut(entity).insert(state);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }
        chase::face(world, entity, goal.0, goal.1);
    }

    // The caster is where it needs to be, so it works at the cast: nothing is
    // spent until the work reaches the skill's cast point.
    if state.phase == 0 && !ready(world, entity, skill) {
        return Processing::state(OrderState::Finished);
    }
    state.phase += 1;
    // At its point or past it, rather than exactly at it: a point read from a
    // stat can shrink under a phase already worked. What has landed is
    // remembered, so it lands once however the numbers move.
    match state.stage {
        CastStage::Working if state.phase >= point => {
            state.stage = CastStage::Holding;
            now(world, player, entity, skill, &def, target);
            // A cast the caster did not survive — its own damage, or a body it
            // stood on being spent — ends here: the queue is out of the world
            // while an order runs, so the loop is what hands the caster its
            // Die order.
            if world.entity(entity).contains::<DyingComponent>() {
                return Processing::finished_dying();
            }
        }
        CastStage::Working => {
            world.entity_mut(entity).insert(state);
            return Processing::state(OrderState::InProcessing);
        }
        // Past its point, the caster is standing through what is left of the
        // cast; the effect landed on the tick the work reached it.
        CastStage::Holding => {}
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

/// What a judged cast is about to do — settled before anything is paid, so a
/// cast that cannot be performed costs nothing.
enum CastPlan {
    /// Act on what the cast was aimed at.
    Act,
    /// Set units of the given type down on the given cells, one each.
    Summon {
        entity_type: EntityTypeId,
        cells: Vec<CellPos>,
    },
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
            Casting::Instant => (1, 1),
            Casting::Delayed { point, period } => (
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
        SkillCaster::Entity { reach, .. } => match reach {
            Reach::Wherever => None,
            Reach::Within(reach) => Some(entity_def::quantity_cells(world, caster, *reach)),
        },
        SkillCaster::Player { .. } => None,
    }
}

/// Casts `skill` from `caster` at `target`, here and now.
///
/// Refuses silently — the caster no longer has the skill ready, the aim
/// resolves to nothing, there is no room for what would be summoned, or the
/// cost cannot be paid — because a cast comes off the wire and a refusal is
/// not the simulation's business to report.
fn now(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    def: &SkillDef,
    target: Option<SkillTarget>,
) {
    let SkillCaster::Entity {
        costs,
        target: _,
        reach: _,
        casting: _,
        effect,
    } = &def.caster
    else {
        unreachable!("a type declares entity casts only, so only one is cast from one")
    };
    if !ready(world, caster, skill) {
        return;
    }
    let Ok(aim) = aim(world, player, caster, skill, target) else {
        return;
    };
    let Some(plan) = judge(world, player, *effect, aim) else {
        return;
    };
    if !cost::can_pay(world, caster, player, costs) {
        return;
    }
    cost::pay(world, caster, player, costs, SpendCause::Skill { skill });
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
    perform(world, player, caster, aim, *effect, plan, spent);
    entity_skills::cast(world, caster, landed_on, skill, def.cooldown);
}

/// Whether the caster has this skill ready and is in a state to cast at all.
pub(super) fn ready(world: &World, caster: Entity, skill: SkillId) -> bool {
    let has_skill = world
        .entity(caster)
        .get::<SkillsComponent>()
        .is_some_and(|skills| skills.ready(skill));
    has_skill && orders::requires_operating(world, caster).is_ok()
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
}

/// Resolves what `caster`'s cast of `skill` acts on when aimed at `target`, or
/// why it cannot take it — an entity that cannot carry the buff the cast
/// applies included.
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
    let def = world
        .resource::<ContentRegistry>()
        .skill_def(skill)
        .expect("a skill id comes from the registry");
    let SkillCaster::Entity {
        target: cast_target,
        effect,
        ..
    } = &def.caster
    else {
        panic!("aim is asked of a skill an entity casts");
    };
    let effect = *effect;
    match cast_target {
        EntityCastTarget::Caster => {
            takes(world, caster, effect)?;
            Ok(CastAim::Entity(caster))
        }
        EntityCastTarget::Position => {
            let Some(SkillTarget::Position(position)) = target else {
                return Err(AimRefusal::Unaimed);
            };
            let cell = CellPos::from(position);
            if world.resource::<Map>().contains(cell) {
                Ok(CastAim::Cell(cell))
            } else {
                Err(AimRefusal::OffMap)
            }
        }
        EntityCastTarget::Fallen { kinds } => {
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
        EntityCastTarget::Standing { side, kinds } => {
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
fn takes(world: &World, entity: Entity, effect: EntityCastEffect) -> Result<(), AimRefusal> {
    match effect {
        EntityCastEffect::ApplyBuff(buff) => {
            if entity_def::can_carry(world, entity, buff) {
                Ok(())
            } else {
                Err(AimRefusal::Uncarried)
            }
        }
        EntityCastEffect::Heal(_) => {
            if entity_def::has_pool(world, entity, PoolId::HEALTH) {
                Ok(())
            } else {
                Err(AimRefusal::Uncarried)
            }
        }
        EntityCastEffect::RemoveBuff(_)
        | EntityCastEffect::Damage(_)
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Summon { .. } => Ok(()),
    }
}

/// Whether `kinds` names what `entity` is.
fn names(world: &World, entity: Entity, kinds: &Kinds) -> bool {
    kinds.admits(entity_def::of(world, entity))
}

/// Settles what the cast will do, or `None` when it cannot be done at all.
///
/// Only a summon can fail here: the units it calls up need ground to stand on
/// and headroom in the owner's supply, and a cast that cannot set every one of
/// them down does nothing rather than half of it.
fn judge(
    world: &World,
    player: PlayerId,
    effect: EntityCastEffect,
    aim: CastAim,
) -> Option<CastPlan> {
    let (entity_type, count) = match effect {
        EntityCastEffect::Summon { entity_type, count } => (entity_type, count),
        EntityCastEffect::ApplyBuff(_)
        | EntityCastEffect::RemoveBuff(_)
        | EntityCastEffect::Damage(_)
        | EntityCastEffect::Heal(_)
        | EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. } => return Some(CastPlan::Act),
    };
    let def = world.resource::<ContentRegistry>().def(entity_type);
    let count = count as usize;
    if !supply::allows_all(world, player, def, count) {
        return None;
    }
    let location_def = def
        .location
        .expect("validated content summons a type that stands somewhere");
    let (around, around_size) = match aim {
        CastAim::Cell(cell) => (cell, CellSize::ONE),
        CastAim::Entity(target) | CastAim::Remains(target) => {
            let (position, size) = entity_def::footprint(world, target);
            (CellPos::from(position), size)
        }
    };
    let cells =
        world
            .resource::<Map>()
            .find_placements_near(around, around_size, &location_def, count);
    (cells.len() == count).then_some(CastPlan::Summon { entity_type, cells })
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

/// Carries out a judged cast at `aim`.
fn perform(
    world: &mut World,
    player: PlayerId,
    caster: Entity,
    aim: CastAim,
    effect: EntityCastEffect,
    plan: CastPlan,
    from_remains: Option<SimulationId>,
) {
    let caster_id = entity_def::simulation_id(world, caster);
    match plan {
        CastPlan::Summon { entity_type, cells } => {
            let type_name = world
                .resource::<ContentRegistry>()
                .def(entity_type)
                .name
                .clone();
            // What comes out of a body remembers the body it came from; what is
            // called up out of nothing names only its caster.
            for cell in cells {
                let position = FixedUVec2::from(cell);
                // What a cast calls up is as new as anything trained: its
                // field sources start at their initial reach.
                let reach = FieldReach::Initial;
                match from_remains {
                    Some(from) => spawn::spawn_raised(
                        world,
                        &type_name,
                        position,
                        Some(player),
                        reach,
                        caster_id,
                        from,
                    ),
                    None => spawn::spawn_summoned(
                        world,
                        &type_name,
                        position,
                        Some(player),
                        reach,
                        caster_id,
                    ),
                };
            }
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
    effect: EntityCastEffect,
) {
    // The two effects that act on a patch of ground rather than on a thing:
    // both read the aim as a cell, whether it was aimed there or at something
    // standing there.
    match effect {
        EntityCastEffect::Field {
            field,
            radius,
            action,
        } => {
            let center = aimed_cell(world, aim);
            fields::apply_action(world, player, field, center, radius, action);
            return;
        }
        EntityCastEffect::Watch {
            radius,
            duration,
            detection,
        } => {
            let center = aimed_cell(world, aim);
            let caster_id = entity_def::simulation_id(world, caster);
            watches::open(
                world, player, caster_id, center, radius, duration, detection,
            );
            return;
        }
        EntityCastEffect::ApplyBuff(_)
        | EntityCastEffect::RemoveBuff(_)
        | EntityCastEffect::Damage(_)
        | EntityCastEffect::Heal(_)
        | EntityCastEffect::Summon { .. } => {}
    }
    let target = match aim {
        CastAim::Entity(target) => target,
        CastAim::Cell(_) | CastAim::Remains(_) => {
            unreachable!(
                "registration pairs a cell with a field or watch, and a body with a summon"
            )
        }
    };
    match effect {
        EntityCastEffect::ApplyBuff(id) => match buffs::apply_entity_buff(world, target, id) {
            Bearing::Borne => {}
            Bearing::Uncarried => {
                unreachable!("the aim admits only a target that can carry the buff")
            }
        },
        EntityCastEffect::RemoveBuff(id) => {
            // A target that does not bear the buff loses nothing.
            if entity_def::bears(world, target, id) {
                buffs::remove_entity_buff(world, target, id);
            }
        }
        EntityCastEffect::Damage(amount) => {
            // Skill damage bypasses armor, like an ability rather than a
            // weapon — and denies nothing: what it kills leaves whatever its
            // type leaves.
            let caster_id = entity_def::simulation_id(world, caster);
            damage::apply(world, caster_id, target, amount, Slain::Remains);
        }
        EntityCastEffect::Heal(amount) => pools::restore(world, target, PoolId::HEALTH, amount),
        EntityCastEffect::Field { .. }
        | EntityCastEffect::Watch { .. }
        | EntityCastEffect::Summon { .. } => {
            unreachable!("handled above")
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

/// The footprint the caster closes on, or `None` when the aim is gone.
fn goal(
    world: &World,
    player: PlayerId,
    caster: Entity,
    skill: SkillId,
    target: Option<SkillTarget>,
) -> Option<(FixedUVec2, CellSize)> {
    match aim(world, player, caster, skill, target).ok()? {
        CastAim::Entity(target) | CastAim::Remains(target) => {
            Some(entity_def::footprint(world, target))
        }
        CastAim::Cell(cell) => Some((FixedUVec2::from(cell), CellSize::ONE)),
    }
}
