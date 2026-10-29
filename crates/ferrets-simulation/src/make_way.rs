//! Who makes way for what needs the cells a body stands on — a walk passing
//! through, or a body placed there — and how a body asked to make way sets
//! off.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{
    components::{
        movement::MoveComponent,
        order_queue::OrderQueueComponent,
        patience::{Patience, Waiting},
        stance::{Stance, StanceComponent},
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    game_loop::orders,
    landing::{self, Bound, ComesFrom, Lander},
    map::Map,
    movement_model::{self, Settling},
    occupancy::GridMark,
    order::Order,
    session::{GameSession, player_id::PlayerId},
    spawn,
};

/// How many ticks a placement waits on bodies making way off its cells before
/// it gives up: two seconds.
const PATIENCE: u32 = 40;

/// Whether a body standing where something is needed gives the cells up when
/// asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Yielding {
    /// It walks off when asked.
    Yields,
    /// It stays where it is.
    Holds,
}

/// What a body in the way is asked to clear.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Need<'a> {
    /// A walk passing through.
    Passage {
        /// The cells it still enters on its way.
        ahead: &'a [CellPos],
        /// The cell it steps from, and the one it steps to: the way it heads.
        heading: (CellPos, CellPos),
    },
    /// Bodies placed on the cells: the footprints they take.
    Placement(&'a [CellRect]),
}

/// What stands between a placement and the ground it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Clearance {
    /// Nothing: the ground takes it as it stands.
    Clear,
    /// These movers: the ground takes it once they walk off, each one that
    /// yields once asked, the rest already on their way.
    Clears(Vec<Entity>),
    /// Something that will not move, or ground that refuses it whoever moves.
    Held,
}

/// What came of setting the movers on a footprint down off it at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shoved {
    /// Every one of them stands off it now.
    Cleared,
    /// Nobody stands on it, one holds, the ground refuses what needs it with
    /// them set aside, or one finds no cell off it; nobody was moved.
    Held,
}

/// How a body asked to make way answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    /// It walks to a cell clear of what is needed.
    Leaves,
    /// No cell it walks to is clear of it.
    Boxed,
}

/// Whether `occupant` gives up the cells it stands on when `side` asks: it
/// yields when it is `side`'s own or allied, a mover operating on the grid,
/// idle, not walking, not dying and not standing its ground; it holds
/// otherwise, whatever is asked of it.
///
/// Panics if `occupant` is not a simulation entity.
pub(crate) fn makes_way(world: &World, occupant: Entity, side: Option<PlayerId>) -> Yielding {
    if allied(world, occupant, side) && gives_way(world, occupant) {
        Yielding::Yields
    } else {
        Yielding::Holds
    }
}

/// Whether `mover` clears ground `side` needs: it yields when asked (see
/// [`makes_way`]), or it is an own or allied walker already on its way,
/// operating and not dying.
///
/// Panics if `mover` is not a simulation entity.
pub fn clears(world: &World, mover: Entity, side: Option<PlayerId>) -> bool {
    allied(world, mover, side) && (gives_way(world, mover) || walks_off(world, mover))
}

/// What stands between a placement on `footprint`, on `layers`, and its
/// ground, for `side`: [`Clearance::Clear`] when `takes` judges the ground to
/// take it as it stands; [`Clearance::Clears`] when it takes it with the movers
/// standing there set aside — their marks lifted and the bodies ignored —
/// every one of them either yielding to `side` or an own or allied walker
/// already on its way; [`Clearance::Held`] otherwise.
/// The movers are read in id order, none of `except`.
pub(crate) fn clearance(
    world: &World,
    side: Option<PlayerId>,
    footprint: CellRect,
    layers: LayerMask,
    except: &[Entity],
    takes: impl Fn(&[GridMark], &[Entity]) -> bool,
) -> Clearance {
    if takes(&[], &[]) {
        return Clearance::Clear;
    }
    let mut marks: Vec<GridMark> = Vec::new();
    let mut bodies: Vec<Entity> = Vec::new();
    for mover in movers_on(world, footprint, layers, except) {
        if !clears(world, mover, side) {
            return Clearance::Held;
        }
        marks.extend(entity_def::grid_mark(world, mover));
        bodies.push(mover);
    }
    if !bodies.is_empty() && takes(&marks, &bodies) {
        Clearance::Clears(bodies)
    } else {
        Clearance::Held
    }
}

/// Asks `occupant`, for `side`, to make way for `need`: a walk to the nearest
/// cell it walks to whose footprint is clear of the cells needed — for a
/// passage, one not back the way the walk comes, which it would have to cross
/// the walk to reach — pushed in front of its queue and started at once.
/// [`Asked::Boxed`] when no such cell lies within the set-down radius, and
/// nothing is asked.
///
/// Panics for an occupant that holds when `side` asks (see [`makes_way`]).
pub(crate) fn ask(
    world: &mut World,
    occupant: Entity,
    side: Option<PlayerId>,
    need: Need,
) -> Asked {
    match send_off(world, occupant, side, need, &[]) {
        Some(_) => Asked::Leaves,
        None => Asked::Boxed,
    }
}

/// Sets every mover standing over `footprint`, on any of `layers`, but
/// `except`, down at once off it, each on the nearest cell the set-down search
/// gives it clear of the footprint and of the cells the others are given:
/// [`Shoved::Cleared`] when every one of them clears for `side` (see
/// [`clears`]), `takes` judges the ground to take what needs it with them set
/// aside — their marks lifted and the bodies ignored — and each finds such a
/// cell; [`Shoved::Held`] otherwise, and nobody is moved. A walker is settled
/// off any crossing first, and its walk plans again from where it is set
/// down.
pub(crate) fn shove_off(
    world: &mut World,
    side: Option<PlayerId>,
    footprint: CellRect,
    layers: LayerMask,
    except: &[Entity],
    takes: impl Fn(&World, &[GridMark], &[Entity]) -> bool,
) -> Shoved {
    let movers = movers_on(world, footprint, layers, except);
    if movers.is_empty() || !movers.iter().all(|&mover| clears(world, mover, side)) {
        return Shoved::Held;
    }
    let marks: Vec<GridMark> = movers
        .iter()
        .filter_map(|&mover| entity_def::grid_mark(world, mover))
        .collect();
    if !takes(world, &marks, &movers) {
        return Shoved::Held;
    }
    let mut moves: Vec<(Entity, CellPos)> = Vec::new();
    let mut given: Vec<CellRect> = Vec::new();
    for &mover in &movers {
        let around = entity_def::standing_rect(world, mover);
        let lander = Lander {
            def: entity_def::of(world, mover),
            owner: entity_def::owner(world, mover),
            from: ComesFrom::of(world, mover),
            bound: Bound::Radius,
            around,
            toward: around,
        };
        let lifted: Vec<GridMark> = entity_def::grid_mark(world, mover).into_iter().collect();
        let ignored: Vec<Entity> = except.iter().copied().chain([mover]).collect();
        let Some(cell) = landing::set_down(world, lander, &lifted, &ignored, |_, placed| {
            !placed.intersects(footprint) && !given.iter().any(|taken| taken.intersects(placed))
        }) else {
            return Shoved::Held;
        };
        given.push(CellRect::new(cell, entity_def::location_size(world, mover)));
        moves.push((mover, cell));
    }
    // A walker caught mid-step settles first; once set down, its walk plans
    // again from where it stands.
    for &(mover, _) in &moves {
        match movement_model::settle_crossing(world, mover) {
            Settling::Settled | Settling::AtRest => {}
        }
    }
    spawn::relocate(world, &moves);
    for &(mover, _) in &moves {
        if let Some(mut walk) = world.entity_mut(mover).get_mut::<MoveComponent>() {
            walk.repath_avoiding_claims();
        }
    }
    Shoved::Cleared
}

/// Asks the own and allied movers that alone keep the body `lander` builds
/// from landing, for `side`, off the nearest footprint they keep it from (see
/// [`blocking_landing`], over what `lifted` marks and ignoring `ignored`).
/// [`Asked::Boxed`] when none would clear one, and nothing is asked.
pub(crate) fn clear_landing(
    world: &mut World,
    side: Option<PlayerId>,
    lander: impl Fn(&World) -> Lander<'_>,
    lifted: &[GridMark],
    ignored: &[Entity],
) -> Asked {
    match blocking_landing(world, side, lander(world), lifted, ignored) {
        Some((footprint, bodies)) => {
            let asks: Vec<(Entity, CellRect)> =
                bodies.iter().map(|&body| (body, footprint)).collect();
            ask_each_off(world, side, &asks)
        }
        None => Asked::Boxed,
    }
}

/// Counts a tick of a placement waiting, for `side`, on `bodies` making way
/// off the footprint each stands over: [`Waiting::GivesUp`] once `patience`
/// has waited more than [`PATIENCE`] ticks, and nobody is asked; otherwise
/// each that yields is asked off its footprints (see [`ask`]) and
/// [`Waiting::Waits`].
pub(crate) fn wait_off(
    world: &mut World,
    side: Option<PlayerId>,
    bodies: &[(Entity, CellRect)],
    patience: &mut Patience,
) -> Waiting {
    match patience.wait(Some(PATIENCE)) {
        Waiting::Waits => {}
        Waiting::GivesUp => return Waiting::GivesUp,
    }
    match ask_each_off(world, side, bodies) {
        Asked::Leaves | Asked::Boxed => {}
    }
    Waiting::Waits
}

/// Where `lander` would land, for `side`, once the movers that clear for it
/// walked off — those that yield, and own and allied walkers already on their
/// way: the footprint [`landing::set_down`] finds over what `lifted` marks and
/// clear of every ground holder but `ignored`, with those movers set aside and
/// on at least one of them, and the ones standing on it that yield, to be
/// asked. `None` when no footprint within its bound would be clear of all but
/// them.
fn blocking_landing(
    world: &World,
    side: Option<PlayerId>,
    lander: Lander,
    lifted: &[GridMark],
    ignored: &[Entity],
) -> Option<(CellRect, Vec<Entity>)> {
    let location = lander
        .def
        .location
        .expect("only a type that stands somewhere lands");
    let layers = location.occupies();
    let map = world.resource::<Map>();
    let whole = CellRect::new(
        CellPos::new(0, 0),
        CellSize::new(map.nav_grid().width(), map.nav_grid().height()),
    );
    let clearing: Vec<(Entity, CellRect, Option<GridMark>)> =
        movers_on(world, whole, layers, ignored)
            .into_iter()
            .filter(|&mover| clears(world, mover, side))
            .map(|mover| {
                let mark = entity_def::grid_mark(world, mover);
                let held = match mark {
                    Some(mark) => mark.footprint,
                    None => entity_def::standing_rect(world, mover),
                };
                (mover, held, mark)
            })
            .collect();
    if clearing.is_empty() {
        return None;
    }
    let lifted: Vec<GridMark> = lifted
        .iter()
        .copied()
        .chain(clearing.iter().filter_map(|&(_, _, mark)| mark))
        .collect();
    let ignored: Vec<Entity> = ignored
        .iter()
        .copied()
        .chain(clearing.iter().map(|&(mover, _, _)| mover))
        .collect();
    let cell = landing::set_down(world, lander, &lifted, &ignored, |_, footprint| {
        clearing
            .iter()
            .any(|(_, held, _)| held.intersects(footprint))
    })?;
    let footprint = CellRect::new(cell, location.size());
    // Every mover in `clearing` is `side`'s own or allied already.
    let on_it = clearing
        .into_iter()
        .filter(|&(mover, held, _)| held.intersects(footprint) && gives_way(world, mover))
        .map(|(mover, _, _)| mover)
        .collect();
    Some((footprint, on_it))
}

/// Asks each body of `asks` that yields to `side` off every footprint paired
/// with it, once, in the order the bodies first appear, each to a cell clear of
/// the ones those before it walk to. [`Asked::Leaves`] when one of them sets
/// off, [`Asked::Boxed`] when none does.
fn ask_each_off(world: &mut World, side: Option<PlayerId>, asks: &[(Entity, CellRect)]) -> Asked {
    let mut bodies: Vec<(Entity, Vec<CellRect>)> = Vec::new();
    for &(body, footprint) in asks {
        match bodies.iter_mut().find(|(asked, _)| *asked == body) {
            Some((_, footprints)) => footprints.push(footprint),
            None => bodies.push((body, vec![footprint])),
        }
    }
    let mut given: Vec<CellRect> = Vec::new();
    for (body, footprints) in bodies {
        match makes_way(world, body, side) {
            Yielding::Yields => {
                given.extend(send_off(
                    world,
                    body,
                    side,
                    Need::Placement(&footprints),
                    &given,
                ));
            }
            Yielding::Holds => {}
        }
    }
    if given.is_empty() {
        Asked::Boxed
    } else {
        Asked::Leaves
    }
}

/// The movers standing on the grid over `footprint`, on any of `layers`, but
/// `except`, in id order: those claiming cells by the footprint they mark, the
/// rest by the cells they stand across.
fn movers_on(
    world: &World,
    footprint: CellRect,
    layers: LayerMask,
    except: &[Entity],
) -> Vec<Entity> {
    world
        .resource::<EntityIndex>()
        .alive_iter()
        .map(|(_, &entity)| entity)
        .filter(|&entity| {
            if except.contains(&entity) || !entity_def::stands_on_grid(world, entity) {
                return false;
            }
            let def = entity_def::of(world, entity);
            let Some(location) = def.location else {
                return false;
            };
            def.can_move()
                && location.occupies() & layers != LayerMask::EMPTY
                && held_rect(world, entity).intersects(footprint)
        })
        .collect()
}

/// The cells a mover on the grid holds: the footprint it marks when it claims
/// its cells, else the cells it stands across.
fn held_rect(world: &World, mover: Entity) -> CellRect {
    match entity_def::grid_mark(world, mover) {
        Some(mark) => mark.footprint,
        None => entity_def::standing_rect(world, mover),
    }
}

/// Sends `occupant` off for `side` as [`ask`] does, its walk goal's footprint
/// also clear of each of `given`: the footprint it walks to, or `None` when no
/// cell takes it, and nothing is asked.
///
/// Panics for an occupant that holds when `side` asks (see [`makes_way`]).
fn send_off(
    world: &mut World,
    occupant: Entity,
    side: Option<PlayerId>,
    need: Need,
    given: &[CellRect],
) -> Option<CellRect> {
    assert_eq!(
        makes_way(world, occupant, side),
        Yielding::Yields,
        "only a body that yields is asked to make way"
    );
    let owner = entity_def::owner(world, occupant);
    let footprint = entity_def::standing_rect(world, occupant);
    let lifted: Vec<_> = entity_def::grid_mark(world, occupant).into_iter().collect();
    let lander = Lander {
        def: entity_def::of(world, occupant),
        owner,
        from: ComesFrom::of(world, occupant),
        bound: Bound::Radius,
        around: footprint,
        toward: footprint,
    };
    let anchor = footprint.origin;
    let clear = |cell: CellPos, footprint: CellRect| {
        let clear_of_need = match need {
            Need::Passage {
                ahead,
                heading: (from, to),
            } => {
                let along = (cell.x as i64 - anchor.x as i64) * (to.x as i64 - from.x as i64)
                    + (cell.y as i64 - anchor.y as i64) * (to.y as i64 - from.y as i64);
                along >= 0 && !ahead.iter().any(|&cell| footprint.contains(cell))
            }
            Need::Placement(rects) => !rects.iter().any(|rect| footprint.intersects(*rect)),
        };
        clear_of_need && !given.iter().any(|taken| taken.intersects(footprint))
    };
    let cell = landing::walk_goal(world, lander, &lifted, clear)?;
    let mut queue = world
        .entity_mut(occupant)
        .take::<OrderQueueComponent>()
        .expect("an idle body keeps its order queue");
    queue.push_front(Order::Move {
        target: FixedUVec2::from(cell),
        size: CellSize::ONE,
        range: 0,
    });
    // The walk is prepared here: the tick's own prepare phase may have passed.
    let operation = entity_def::operation(world, occupant);
    orders::prepare_front(occupant, operation, &mut queue, world);
    world.entity_mut(occupant).insert(queue);
    Some(CellRect::new(
        cell,
        entity_def::location_size(world, occupant),
    ))
}

/// Whether `body` is `side`'s own or allied.
fn allied(world: &World, body: Entity, side: Option<PlayerId>) -> bool {
    world
        .resource::<GameSession>()
        .owners_allied(side, entity_def::owner(world, body))
}

/// Whether `occupant`, whoever's side it is on, gives up its cells when
/// asked: a mover operating on the grid, idle, not walking, not dying and not
/// standing its ground.
fn gives_way(world: &World, occupant: Entity) -> bool {
    // The cheap reads first: most bodies asked about are busy.
    entity_def::idle(world, occupant)
        && !entity_def::walking(world, occupant)
        && entity_def::of(world, occupant).can_move()
        && entity_def::stands_on_grid(world, occupant)
        && !entity_def::dying(world, occupant)
        && operating(world, occupant)
        && !standing_ground(world, occupant)
}

/// Whether `mover`, whoever's side it is on, is walking off on its own:
/// walking, operating and not dying.
fn walks_off(world: &World, mover: Entity) -> bool {
    entity_def::walking(world, mover) && !entity_def::dying(world, mover) && operating(world, mover)
}

/// Whether `body` is operating: not rising, disabled or stranded.
fn operating(world: &World, body: Entity) -> bool {
    match entity_def::operation(world, body) {
        Operation::Operating => true,
        Operation::Rising | Operation::Disabled(_) | Operation::Stranded => false,
    }
}

/// Whether `body` stands its ground.
fn standing_ground(world: &World, body: Entity) -> bool {
    world.entity(body).get::<StanceComponent>().is_some_and(
        |StanceComponent(stance)| match stance {
            Stance::StandGround => true,
            Stance::Flee | Stance::HoldFire | Stance::Defend => false,
        },
    )
}
