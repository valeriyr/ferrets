//! Slopes: bands of cells across a change of level, ramps and cliffs.

use serde::{Deserialize, Serialize};

/// What a cell is at a change of level: part of a band straddling it, its
/// lower cells at the low level and its upper cells at the high level,
/// rising the way its direction says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slope {
    /// Part of a ramp, climbed through the edge its direction crosses.
    Ramp(Direction),
    /// Part of a cliff, crossed only by a mover that crosses cliffs.
    Cliff(Direction),
}

impl Slope {
    /// The way the band rises.
    pub fn rises(self) -> Direction {
        match self {
            Self::Ramp(direction) | Self::Cliff(direction) => direction,
        }
    }

    /// Whether `other` is a slope of the same kind, rising any way.
    pub fn is_same_kind(self, other: Self) -> bool {
        match (self, other) {
            (Self::Ramp(_), Self::Ramp(_)) | (Self::Cliff(_), Self::Cliff(_)) => true,
            (Self::Ramp(_), Self::Cliff(_)) | (Self::Cliff(_), Self::Ramp(_)) => false,
        }
    }
}

/// One of the eight ways from a cell to a neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Toward the row above, `y - 1`.
    North,
    /// Toward the row above and the column right.
    NorthEast,
    /// Toward the column right, `x + 1`.
    East,
    /// Toward the row below and the column right.
    SouthEast,
    /// Toward the row below, `y + 1`.
    South,
    /// Toward the row below and the column left.
    SouthWest,
    /// Toward the column left, `x - 1`.
    West,
    /// Toward the row above and the column left.
    NorthWest,
}

impl Direction {
    /// Every cardinal direction, in clockwise order from north.
    pub const CARDINAL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// The direction of a step of `(dx, dy)`, one cell along each axis at
    /// most.
    ///
    /// Panics for a step that stays put or goes further than a neighbor.
    pub fn of_step(dx: i32, dy: i32) -> Self {
        match (dx, dy) {
            (0, -1) => Self::North,
            (1, -1) => Self::NorthEast,
            (1, 0) => Self::East,
            (1, 1) => Self::SouthEast,
            (0, 1) => Self::South,
            (-1, 1) => Self::SouthWest,
            (-1, 0) => Self::West,
            (-1, -1) => Self::NorthWest,
            other => panic!("a step {other:?} goes to no neighbor"),
        }
    }

    /// The `(dx, dy)` of a step this way.
    pub fn step(self) -> (i32, i32) {
        match self {
            Self::North => (0, -1),
            Self::NorthEast => (1, -1),
            Self::East => (1, 0),
            Self::SouthEast => (1, 1),
            Self::South => (0, 1),
            Self::SouthWest => (-1, 1),
            Self::West => (-1, 0),
            Self::NorthWest => (-1, -1),
        }
    }

    /// The way back.
    pub fn opposite(self) -> Self {
        let (dx, dy) = self.step();
        Self::of_step(-dx, -dy)
    }

    /// Whether this is one of the four cardinal directions.
    pub fn is_cardinal(self) -> bool {
        let (dx, dy) = self.step();
        dx == 0 || dy == 0
    }

    /// Whether a band rising this way rises across the cardinal `edge`: the
    /// direction itself, or a diagonal one leaning that way.
    pub fn crosses(self, edge: Self) -> bool {
        let ((dx, dy), (ex, ey)) = (self.step(), edge.step());
        (ex != 0 && dx == ex) || (ey != 0 && dy == ey)
    }
}
