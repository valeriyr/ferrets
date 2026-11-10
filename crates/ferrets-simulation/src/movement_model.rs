//! Selectable movement execution models.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, location::Location};
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::{components::movement::MoveComponent, entity_def, map::Map, occupancy::GridMark};

/// How a game's units occupy space and resolve blocking between each other.
///
/// The model is fixed game configuration, identical on every peer for the
/// whole session. It selects the movement execution the simulation runs;
/// long-range pathfinding is shared by every model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MovementModel {
    /// Units claim whole cells, one claimant per cell per layer, and move
    /// cell-to-cell in discrete crossings. Positions stay on the cell
    /// lattice.
    #[default]
    Cell,
    /// Units occupy radius circles and resolve contact by pushing each
    /// other apart; positions are unconstrained points. Each body claims the
    /// footprint its position anchors, its claim following it as it moves;
    /// bodies may share cells.
    Continuous,
}

/// How [`settle_crossing`] found a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settling {
    /// It was crossing and now stands on the cell it crossed into.
    Settled,
    /// It was at rest, and stays where it is.
    AtRest,
}

/// Moves a continuous-model body standing on the grid to `to`, its claim
/// following it onto the footprint it is anchored at there.
///
/// Panics under the cell model, for a body that claims cells but stands off
/// the grid, and as [`entity_def::set_location`] does.
pub fn move_body(world: &mut World, entity: Entity, to: Location) {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => panic!("a cell-model body moves crossing by crossing"),
        MovementModel::Continuous => {}
    }
    let held = entity_def::grid_mark(world, entity);
    entity_def::set_location(world, entity, to);
    let Some(from) = held else {
        // A body that claims no cells has no claim to move.
        return;
    };
    claim_onto(world, entity, from, to.anchor());
}

/// Moves the claim of a cell-model `entity`, at rest on the grid, onto the
/// footprint anchored at `to` as its crossing there starts, before its
/// position changes. A body that claims no cells has nothing to move.
///
/// Panics for a body that claims cells but stands off the grid.
pub fn step_claim(world: &mut World, entity: Entity, to: CellPos) {
    let Some(from) = entity_def::grid_mark(world, entity) else {
        return;
    };
    claim_onto(world, entity, from, to);
}

/// Trades the claims of two cell-model bodies at rest swapping cells head-on,
/// before either moves: each takes the footprint the other leaves.
///
/// Panics for a body that claims no cells or stands off the grid, and for
/// two bodies whose claims differ in size or layers.
pub fn swap_claims(world: &mut World, first: Entity, second: Entity) {
    let mark_of = |world: &World, entity: Entity| {
        assert!(
            entity_def::stands_on_grid(world, entity),
            "a body swapping its claim stands on the grid"
        );
        entity_def::grid_mark(world, entity).expect("a body swapping cells claims its own")
    };
    let (first_mark, second_mark) = (mark_of(world, first), mark_of(world, second));
    assert!(
        first_mark.footprint.size == second_mark.footprint.size
            && first_mark.layers == second_mark.layers,
        "bodies swapping cells hold claims of one size on the same layers"
    );
    let mut map = world.resource_mut::<Map>();
    map.unclaim(first_mark);
    map.unclaim(second_mark);
    map.claim(GridMark {
        footprint: second_mark.footprint,
        ..first_mark
    });
    map.claim(GridMark {
        footprint: first_mark.footprint,
        ..second_mark
    });
}

/// Whether `entity` stands between two anchored positions: a crossing is in
/// progress.
///
/// This infers movement state from the position alone, which is only valid
/// under the lattice invariant documented on
/// [`LocationComponent`](crate::components::location::LocationComponent): a
/// body not crossing stands exactly on its anchored position, so any other
/// position can mean nothing but a crossing in progress.
///
/// Panics if `entity` is not a simulation entity, is off the map, or its type
/// declares no location.
pub fn is_mid_crossing(world: &World, entity: Entity) -> bool {
    entity_def::location(world, entity).is_mid_crossing()
}

/// Whether `walker` stands where a point move to `target` ends: `target` held
/// on the map for its size, lined up anchored there under the cell model, its
/// middle exactly there under the continuous one.
pub fn at_point(world: &World, walker: Entity, target: FixedUVec2) -> bool {
    let standing = entity_def::location(world, walker);
    let map = world.resource::<Map>();
    let spot = map.location_clamped(target, standing.size());
    match map.movement_model() {
        MovementModel::Cell => !standing.is_mid_crossing() && standing.anchor() == spot.anchor(),
        MovementModel::Continuous => standing.middle() == spot.middle(),
    }
}

/// Settles a cell-model walker caught mid-crossing onto the cell it is
/// crossing into, the cell its crossing claimed: [`Settling::Settled`];
/// [`Settling::AtRest`] for a body that is not walking, stands on its cell,
/// or walks under the continuous model, left where it is.
///
/// Panics for a walker that stands nowhere.
pub(crate) fn settle_crossing(world: &mut World, entity: Entity) -> Settling {
    let Some(claimed) = world
        .entity(entity)
        .get::<MoveComponent>()
        .and_then(|movement| movement.path.last().copied())
    else {
        return Settling::AtRest;
    };
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell if is_mid_crossing(world, entity) => {
            entity_def::set_anchor(world, entity, claimed);
            Settling::Settled
        }
        MovementModel::Cell | MovementModel::Continuous => Settling::AtRest,
    }
}

/// Moves `entity`'s mark `from` onto the footprint of its size anchored at
/// `to`.
///
/// Panics for a body standing off the grid.
fn claim_onto(world: &mut World, entity: Entity, from: GridMark, to: CellPos) {
    assert!(
        entity_def::stands_on_grid(world, entity),
        "a body moved with its claim stands on the grid"
    );
    let moved = GridMark {
        footprint: CellRect::new(to, from.footprint.size),
        ..from
    };
    world.resource_mut::<Map>().move_claim(from, moved);
}
