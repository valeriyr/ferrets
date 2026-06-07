//! In-flight movement state and content-defined movement properties for simulation entities.

use bevy_ecs::prelude::*;

use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_pos::NavPos;

/// Content-defined movement properties for an entity type.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveStaticData {
    /// Movement speed in grid units per tick (fixed-point, supports fractional values).
    speed: FixedU64,
}

/// Per-entity in-flight movement state.
#[derive(Component, Debug)]
pub struct MoveComponent {
    /// Remaining path waypoints. The last element is the cell the entity is currently
    /// moving toward (or will move toward next if at rest). Popped on arrival.
    pub path: Vec<NavPos>,
    /// The cell the entity departed from when the current crossing started.
    /// Used by the renderer for position interpolation.
    pub moving_from: NavPos,
    /// Ticks remaining to wait for a blocked cell to clear before recalculating the path.
    pub wait_ticks: u32,
}

impl MoveComponent {
    /// Trims the path to only `path.last()` so the entity stops after reaching it.
    ///
    /// Whether mid-crossing or at rest, `path.last()` is always the immediate next
    /// target. Everything before it is discarded.
    pub fn leave_only_current_target(&mut self) {
        if let Some(current) = self.path.last().copied() {
            self.path.clear();
            self.path.push(current);
        }
    }
}

impl MoveStaticData {
    /// Creates a new `MoveStaticData` with the given data.
    pub fn new(speed: FixedU64) -> Self {
        Self { speed }
    }

    /// Returns the entity's movement speed in grid units per tick.
    pub fn speed(&self) -> FixedU64 {
        self.speed
    }
}

impl MoveComponent {
    /// Creates a new `MoveComponent` with the given entity position.
    #[inline]
    pub fn new(from: NavPos) -> Self {
        Self {
            path: Vec::new(),
            moving_from: from,
            wait_ticks: 0,
        }
    }
}
