//! In-flight movement state for simulation entities.

use bevy_ecs::prelude::*;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_pathfinder::hpa::{Crossing, PlanTarget};

/// Per-entity in-flight movement state.
#[derive(Component, Debug)]
pub struct MoveComponent {
    /// Remaining cells of the current refined segment. The last element is
    /// the cell the entity is currently moving toward (or will move toward
    /// next if at rest). Popped on arrival.
    pub path: Vec<CellPos>,
    /// The crossings of the planned corridor still ahead, last first; the
    /// next segment refines from them when the path runs out.
    pub corridor: Vec<Crossing>,
    /// The destination the current plan leads to — the requested goal or its
    /// nearest reachable repair. `None` before the first plan, and for flat
    /// plans, which need no refinement.
    pub plan: Option<PlanTarget>,
    /// Plan the next segment with unit claims as obstacles — set after a
    /// crossing stayed blocked, so the walk routes around parked units.
    pub avoid_claims: bool,
    /// Whether the current blockage already spent its local detour, so the
    /// next escalation is a full repath.
    pub detoured: bool,
    /// Consecutive blockage escalations without a completed crossing. Grows
    /// the acceptance range so crowds settle into a ring, and bounds how
    /// long a walk grinds before giving up. Reset by any progress.
    pub frustration: u32,
    /// The cell the entity departed from when the current crossing started.
    pub moving_from: CellPos,
    /// Ticks remaining to wait for a blocked cell to clear before recalculating the path.
    pub wait_ticks: u32,
    /// The closest straight-line distance to the pursued waypoint the walk
    /// has reached so far; reset whenever the waypoint changes. A continuous
    /// walk counts as progressing only while it keeps setting new records —
    /// a body churned around a crowd drifts without getting anywhere, and
    /// must escalate instead, while a long way round keeps consuming
    /// waypoints and never escalates.
    pub best_distance: FixedU64,
}

impl MoveComponent {
    /// Trims the plan to only `path.last()` so the entity stops after
    /// reaching it.
    ///
    /// Whether mid-crossing or at rest, `path.last()` is always the immediate next
    /// target. Everything before it is discarded.
    pub fn leave_only_current_target(&mut self) {
        if let Some(current) = self.path.last().copied() {
            self.path.clear();
            self.path.push(current);
        }
        self.corridor.clear();
        self.plan = None;
    }

    /// Creates a new `MoveComponent` with the given entity position.
    #[inline]
    pub fn new(from: CellPos) -> Self {
        Self {
            path: Vec::new(),
            corridor: Vec::new(),
            plan: None,
            avoid_claims: false,
            detoured: false,
            frustration: 0,
            moving_from: from,
            wait_ticks: 0,
            best_distance: FixedU64::MAX,
        }
    }
}
