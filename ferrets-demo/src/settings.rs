//! Pre-game options: one resource the main menu edits, stamped by every
//! entry path onto the game it configures.

use bevy::prelude::*;
use ferrets_geometry::projection::Projection;
use ferrets_simulation::movement_model::MovementModel;

/// The demo's pre-game options. They shape the next game only — the game in
/// progress reads its own map, never this resource.
#[derive(Resource, Clone, Copy)]
pub struct Settings {
    /// How units occupy space and resolve blocking between each other.
    pub movement_model: MovementModel,
    /// The map's distance metric together with the way it is drawn.
    pub view: View,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            movement_model: MovementModel::Continuous,
            view: View::IsometricSquare,
        }
    }
}

/// The map's distance metric together with the way the world is drawn. The
/// isometric metric has two established looks — the classic diamond and the
/// flat square grid — while the orthogonal (Euclidean) metric reads right
/// only on a square grid.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The Chebyshev metric drawn as the classic isometric diamond.
    IsometricDiamond,
    /// The Chebyshev metric drawn as a flat square grid.
    IsometricSquare,
    /// The Euclidean metric on a square grid.
    Orthogonal,
}

impl View {
    /// The distance metric this view plays with.
    pub fn projection(self) -> Projection {
        match self {
            View::IsometricDiamond | View::IsometricSquare => Projection::Isometric,
            View::Orthogonal => Projection::Orthogonal,
        }
    }

    /// Whether the world draws as diamonds.
    pub fn diamond(self) -> bool {
        match self {
            View::IsometricDiamond => true,
            View::IsometricSquare | View::Orthogonal => false,
        }
    }
}
