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
    /// The pursued waypoint is a lattice point the walk regains after being
    /// walled off, and pops only on exact arrival: the ordinary half-body
    /// slack would consume it early and leave the body off-lattice, still
    /// pressed into the corner it was regaining from.
    pub regaining: bool,
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

    /// Forgives every blockage escalation: the stall clock, the frustration
    /// ladder, and the spent local detour all reset — the way ahead is clear.
    pub fn forgive(&mut self) {
        self.wait_ticks = 0;
        self.frustration = 0;
        self.detoured = false;
    }

    /// Records a new closest distance to the pursued waypoint. Closing in on
    /// a real waypoint means the way ahead is clear, so every escalation is
    /// forgiven along with it — but closing in on a regained lattice point
    /// proves only that the body can walk *back* to where it was walled off,
    /// so the escalations earned there stand; forgiving them let a walled
    /// walk launder its frustration through every regain round-trip and
    /// grind forever.
    pub fn record_progress(&mut self, distance: FixedU64) {
        self.best_distance = distance;
        if self.regaining {
            self.wait_ticks = 0;
        } else {
            self.forgive();
        }
    }

    /// One blockage escalation: the stall clock restarts and the frustration
    /// ladder climbs a rung. Returns the new rung, for the caller's give-up
    /// budget.
    pub fn escalate(&mut self) -> u32 {
        self.wait_ticks = 0;
        self.frustration += 1;
        self.frustration
    }

    /// Takes up a freshly refined segment, walked front to back; the first
    /// waypoint starts its own progress record.
    pub fn pursue_segment(&mut self, segment: Vec<CellPos>) {
        self.path = segment.into_iter().rev().collect();
        self.best_distance = FixedU64::MAX;
    }

    /// Takes up `waypoint` as the immediate next target, with its own
    /// progress record.
    pub fn pursue(&mut self, waypoint: CellPos) {
        self.path.push(waypoint);
        self.best_distance = FixedU64::MAX;
    }

    /// Replaces the pursued waypoint with a detour that rejoins it, walked
    /// front to back, and marks the blockage's one local detour spent.
    pub fn splice_detour(&mut self, cells: Vec<CellPos>) {
        self.detoured = true;
        self.path.pop();
        self.path.extend(cells.into_iter().rev());
        self.best_distance = FixedU64::MAX;
    }

    /// Holds `lattice` as the immediate waypoint, to be regained exactly: the
    /// pop stays precise (see [`Self::regaining`]) and the escalation state
    /// stays — the body is only being put back where it was walled off.
    ///
    /// A body walled off tick after tick regains the same point each time;
    /// one held waypoint is the intent, not one per attempt, so a repeat of
    /// the waypoint already being regained is not pushed again.
    pub fn regain(&mut self, lattice: CellPos) {
        if !(self.regaining && self.path.last() == Some(&lattice)) {
            self.path.push(lattice);
        }
        self.regaining = true;
        self.best_distance = FixedU64::MAX;
    }

    /// Consumes the reached waypoint; the next one starts its own progress
    /// record. An ordinary waypoint is progress and forgives every
    /// escalation; a regained lattice point is not — it only put the body
    /// back where it was walled off, and forgiving there would let a loop of
    /// wall, regain, wall reset its own give-up clock each pass.
    pub fn consume_waypoint(&mut self) {
        self.path.pop();
        self.best_distance = FixedU64::MAX;
        self.wait_ticks = 0;
        if self.regaining {
            self.regaining = false;
        } else {
            self.frustration = 0;
            self.detoured = false;
        }
    }

    /// Throws the whole plan away and asks the next one to honor unit claims
    /// — the last rung of the blockage ladder.
    pub fn repath_avoiding_claims(&mut self) {
        self.path.clear();
        self.corridor.clear();
        self.plan = None;
        self.avoid_claims = true;
        self.detoured = false;
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
            regaining: false,
        }
    }
}
