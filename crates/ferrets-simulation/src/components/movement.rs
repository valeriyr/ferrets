//! In-flight movement state for simulation entities.

use bevy_ecs::prelude::*;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
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
    /// long a walk grinds before giving up. Reset when progress forgives (see
    /// [`Self::land`] and [`Self::measure`]).
    pub frustration: u32,
    /// The cell the entity departed from when the current crossing started.
    pub moving_from: CellPos,
    /// The walk is being wound down and ends as soon as the step in progress
    /// lands, rather than asking for more of its route. Without it an emptied
    /// path is indistinguishable from a spent corridor leg, and a walk told to
    /// stop simply plans the rest of its journey again.
    pub winding_down: bool,
    /// The body is standing still to come round toward the way it is going, and
    /// keeps standing until its look is near enough to walk on. Latched because
    /// the angle that starts the hold is wider than the one that ends it: without
    /// it, a body between the two could not tell a turn it is part way through
    /// from a look it is merely carrying.
    pub pivoting: bool,
    /// Ticks the walk has gone without progress: the stall clock.
    pub wait_ticks: u32,
    /// The closest straight-line distance to where the walk aims it has
    /// reached so far; `None` until it is first measured after the waypoint
    /// changes.
    pub best_distance: Option<FixedU64>,
    /// What the pursued waypoint is, for how it pops and whether its pop
    /// forgives.
    pub pursued: Pursued,
}

/// What a continuous walk's pursued waypoint is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pursued {
    /// An ordinary waypoint, popped within the pop slack.
    Ordinary,
    /// An anchored position the walk regains after being walled off, popped
    /// only on exact arrival.
    Regaining,
    /// An ordinary waypoint, the one before it popped last tick at this point:
    /// whether the pop forgives waits on where contact left the body.
    Popped(FixedUVec2),
}

/// How a pursued waypoint left the path.
#[derive(Debug, Clone, Copy)]
pub enum Pop {
    /// Reached: the step ended within the pop slack of this point, the
    /// waypoint's aim.
    Reached(FixedUVec2),
    /// Dropped for its holder, a body that does not give way: the step met
    /// it at this point.
    Skipped(FixedUVec2),
}

/// Where contact left a body whose waypoint popped last tick.
#[derive(Debug, Clone, Copy)]
pub enum Landing {
    /// Within the pop slack of where the pop was.
    AtPop,
    /// Handed back beyond the pop slack.
    HandedBack,
}

/// What one measurement of a walk's distance to its pursued waypoint found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    /// The first since the waypoint changed: it only sets the record.
    Baseline,
    /// A new record.
    Closer,
    /// No closer: the stall clock's count after this tick.
    Stalled(u32),
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
        self.winding_down = true;
    }

    /// Forgives every blockage escalation: the stall clock, the frustration
    /// ladder, and the spent local detour all reset — the way ahead is clear.
    pub fn forgive(&mut self) {
        self.wait_ticks = 0;
        self.frustration = 0;
        self.detoured = false;
    }

    /// Measures the walk's distance to the pursued waypoint against its
    /// record. The first measurement after the waypoint changes only sets the
    /// record. A closer one is a new record: it restarts the stall clock, and
    /// on an ordinary waypoint off a detour forgives every escalation as well.
    /// Any other ticks the stall clock on, and the tick's count is returned
    /// as [`Measured::Stalled`].
    ///
    /// Panics if a pop waits unlanded.
    pub fn measure(&mut self, distance: FixedU64) -> Measured {
        let forgives = match self.pursued {
            Pursued::Ordinary => !self.detoured,
            Pursued::Regaining => false,
            Pursued::Popped(_) => panic!("a pop lands before the walk measures its progress"),
        };
        match self.best_distance {
            None => {
                self.best_distance = Some(distance);
                Measured::Baseline
            }
            Some(best) if distance < best => {
                self.best_distance = Some(distance);
                if forgives {
                    self.forgive();
                } else {
                    self.wait_ticks = 0;
                }
                Measured::Closer
            }
            Some(_) => {
                self.wait_ticks += 1;
                Measured::Stalled(self.wait_ticks)
            }
        }
    }

    /// Excuses one tick of not closing in. A tick spent coming round is not a
    /// tick spent going nowhere, but the distance record cannot tell them apart:
    /// only the walk itself knows it turned instead. Narrow on purpose — it can
    /// cancel no more than the tick that just passed, so nothing launders its way
    /// out of the stall clock by turning.
    pub fn excuse(&mut self) {
        self.wait_ticks = self.wait_ticks.saturating_sub(1);
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
        self.best_distance = None;
    }

    /// Throws the rest of the plan away and takes up `waypoint` as the walk's
    /// last, with its own progress record.
    pub fn pursue_last(&mut self, waypoint: CellPos) {
        self.path.clear();
        self.corridor.clear();
        self.plan = None;
        self.path.push(waypoint);
        self.best_distance = None;
    }

    /// Replaces the pursued waypoint with a detour that rejoins it, walked
    /// front to back, and marks the blockage's one local detour spent.
    pub fn splice_detour(&mut self, cells: Vec<CellPos>) {
        self.detoured = true;
        self.path.pop();
        self.path.extend(cells.into_iter().rev());
        self.best_distance = None;
        // The point being regained is what the pop just discarded; the detour's
        // cells are ordinary waypoints, and holding them to an exact hit would
        // strand a body the pushing pass keeps nudging off them.
        self.pursued = Pursued::Ordinary;
    }

    /// Holds `lattice` as the immediate waypoint, to be regained exactly: the
    /// pop stays precise (see [`Pursued::Regaining`]) and the escalation state
    /// stays — the body is only being put back where it was walled off.
    ///
    /// A body walled off tick after tick regains the same point each time;
    /// one held waypoint is the intent, not one per attempt, so a repeat of
    /// the waypoint already being regained is not pushed again.
    pub fn regain(&mut self, lattice: CellPos) {
        if !(matches!(self.pursued, Pursued::Regaining) && self.path.last() == Some(&lattice)) {
            self.path.push(lattice);
        }
        self.pursued = Pursued::Regaining;
        self.best_distance = None;
    }

    /// Takes the pursued waypoint off the path; the next one starts its own
    /// progress record and the stall clock runs on. An ordinary waypoint
    /// reached, or skipped off a detour, waits as [`Pursued::Popped`] for the
    /// next tick's [`Self::land`]; a regained position, or a waypoint skipped
    /// on a detour, forgives nothing.
    ///
    /// Panics if the walk pursues no waypoint, or a pop waits unlanded.
    pub fn pop_waypoint(&mut self, pop: Pop) {
        self.path
            .pop()
            .expect("a popped waypoint is one the walk pursues");
        self.best_distance = None;
        self.pursued = match (self.pursued, pop) {
            (Pursued::Regaining, Pop::Reached(_) | Pop::Skipped(_)) => Pursued::Ordinary,
            (Pursued::Ordinary, Pop::Reached(point)) => Pursued::Popped(point),
            (Pursued::Ordinary, Pop::Skipped(met)) => {
                if self.detoured {
                    Pursued::Ordinary
                } else {
                    Pursued::Popped(met)
                }
            }
            (Pursued::Popped(_), Pop::Reached(_) | Pop::Skipped(_)) => {
                panic!("a pop lands at the top of the next tick")
            }
        };
    }

    /// Settles last tick's pop by where contact left the body: at the pop it
    /// forgives every escalation; handed back it forgives nothing.
    ///
    /// Panics if no pop waits.
    pub fn land(&mut self, landing: Landing) {
        match self.pursued {
            Pursued::Popped(_) => {}
            Pursued::Ordinary | Pursued::Regaining => panic!("only a popped waypoint lands"),
        }
        self.pursued = Pursued::Ordinary;
        match landing {
            Landing::AtPop => self.forgive(),
            Landing::HandedBack => {}
        }
    }

    /// Throws the whole plan away for `route`, a claim-aware route walked front
    /// to back.
    pub fn take_route(&mut self, route: Vec<CellPos>) {
        self.replace_plan();
        self.pursue_segment(route);
    }

    /// Throws the whole plan away and asks the next one to honor unit claims.
    pub fn repath_avoiding_claims(&mut self) {
        self.path.clear();
        self.replace_plan();
        self.avoid_claims = true;
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
            winding_down: false,
            pivoting: false,
            wait_ticks: 0,
            best_distance: None,
            pursued: Pursued::Ordinary,
        }
    }

    /// Drops the plan's corridor and destination for a new plan: its
    /// blockage's detour unspent, its waypoints ordinary — a position being
    /// regained dropped with the plan, a pop waiting to land still waiting —
    /// and no progress record until the new waypoint is measured.
    fn replace_plan(&mut self) {
        self.corridor.clear();
        self.plan = None;
        self.detoured = false;
        self.best_distance = None;
        self.pursued = match self.pursued {
            Pursued::Regaining => Pursued::Ordinary,
            kept @ (Pursued::Ordinary | Pursued::Popped(_)) => kept,
        };
    }
}
