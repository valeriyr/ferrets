//! Internal resolved orders — what entity systems actually execute each tick.

use ferrets_math::fixed_uvec2::FixedUVec2;

/// An order an entity is executing or waiting to execute.
#[derive(Debug, Clone)]
pub enum Order {
    /// Move to a world-space position in simulation coordinates.
    Move { target: FixedUVec2 },
}

impl Order {
    /// If this order is a move order, returns the target position. Otherwise, returns `None`.
    pub fn move_target(&self) -> Option<FixedUVec2> {
        match self {
            Order::Move { target } => Some(*target),
        }
    }
}
