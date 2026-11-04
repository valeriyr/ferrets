//! Selectable movement execution models.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_physics::body;
use serde::{Deserialize, Serialize};

use crate::{
    components::{location::LocationComponent, movement::MoveComponent},
    entity_def,
    map::Map,
    occupancy::OccupancyClass,
};

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
/// following it onto the footprint `to` anchors.
///
/// Panics under the cell model, and for a body that claims cells but stands
/// off the grid.
pub fn move_body(world: &mut World, entity: Entity, to: FixedUVec2) {
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => panic!("a cell-model body moves crossing by crossing"),
        MovementModel::Continuous => {}
    }
    let held = entity_def::grid_mark(world, entity);
    world
        .get_mut::<LocationComponent>(entity)
        .expect("a moving body stands somewhere")
        .position = to;
    let Some(mark) = held else {
        // A passable body claims no cells.
        return;
    };
    assert!(
        entity_def::stands_on_grid(world, entity),
        "a body moved with its claim stands on the grid"
    );
    match mark.class {
        OccupancyClass::Claim => world.resource_mut::<Map>().step_claim(
            mark.layers,
            mark.footprint.size,
            mark.footprint.origin,
            body::anchor(to),
        ),
        OccupancyClass::Static => unreachable!("a body that moves claims its cells"),
    }
}

/// Returns `true` if `position` is between two cell origins (a crossing is in progress).
///
/// This infers movement state from the position alone, which is only valid
/// under the lattice invariant documented on
/// [`LocationComponent::position`](crate::components::location::LocationComponent):
/// entities rest exactly on cell origins (integer coordinates), so a
/// fractional component can mean nothing but a crossing in progress.
pub fn is_mid_crossing(position: FixedUVec2) -> bool {
    let cell = CellPos::from(position);
    position != FixedUVec2::from(cell)
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
    let model = world.resource::<Map>().movement_model();
    let mut location = world
        .get_mut::<LocationComponent>(entity)
        .expect("a walker stands somewhere");
    match model {
        MovementModel::Cell if is_mid_crossing(location.position) => {
            location.position = FixedUVec2::from(claimed);
            Settling::Settled
        }
        MovementModel::Cell | MovementModel::Continuous => Settling::AtRest,
    }
}
