//! Selectable movement execution models.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::{
    components::{location::LocationComponent, movement::MoveComponent},
    map::Map,
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
    /// other apart; positions are unconstrained points. The claim plane is
    /// derived, rebuilt every tick from the cell under each body's center,
    /// so long-range planning still sees where crowds stand.
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
