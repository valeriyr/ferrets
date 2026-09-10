//! Shared vocabulary for how a worker attends a job it takes time to finish.

use ferrets_math::FixedU64;

/// How an attached worker sits in its berth over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BerthStance {
    /// Keeps the berth it took for as long as it works.
    Still,
    /// Works a berth for a while, then walks the group's loop — its berths in
    /// declared order, either way round — to a free berth and works there.
    Circling {
        /// Cells travelled per tick between berths.
        speed: FixedU64,
        /// Ticks worked at a berth before moving on.
        dwell: u32,
    },
    /// Works a berth for a while, then crosses straight to a free berth of the
    /// group and works there.
    Roaming {
        /// Cells travelled per tick between berths.
        speed: FixedU64,
        /// Ticks worked at a berth before moving on.
        dwell: u32,
    },
    /// Circles its own berth without ever leaving it.
    Orbit {
        /// Distance from the berth, in cells.
        radius: FixedU64,
        /// Ticks one circuit takes.
        period: u32,
    },
}

/// What decides how many workers may work one job at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crewing {
    /// The presence's own limit.
    Counted(CrewLimit),
    /// The berths of the job the worker sits in, one worker to a slot.
    Seated,
}

/// How many workers may work one job at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrewLimit {
    /// At most this many, whatever the job.
    Limit(usize),
    /// Any number at all.
    Unlimited,
}

impl CrewLimit {
    /// One worker at a time.
    pub const ONE: Self = CrewLimit::Limit(1);

    /// A limit of `at_once` workers.
    ///
    /// Panics if `at_once` is `0`.
    pub fn limit(at_once: usize) -> Self {
        assert!(at_once > 0, "a crew limit must admit at least one worker");
        CrewLimit::Limit(at_once)
    }

    /// Whether a crew of `workers` exceeds the limit.
    pub fn exceeded_by(&self, workers: usize) -> bool {
        match self {
            CrewLimit::Limit(at_once) => workers > *at_once,
            CrewLimit::Unlimited => false,
        }
    }

    /// Whether the limit admits nobody at all.
    pub fn admits_nobody(&self) -> bool {
        match self {
            CrewLimit::Limit(at_once) => *at_once == 0,
            CrewLimit::Unlimited => false,
        }
    }
}

/// How a worker attaches to a job: which of the job's berth groups it sits in,
/// and how it sits there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The berth group of the job the worker sits in, by name.
    berths: String,
    /// How the worker sits in its berth over time.
    stance: BerthStance,
}

impl Attachment {
    /// Creates a new `Attachment` with the given data.
    ///
    /// Panics if `berths` is empty, or the stance moves at no speed, or orbits
    /// at no distance or with a period of `0`.
    pub fn new(berths: impl Into<String>, stance: BerthStance) -> Self {
        let berths = berths.into();
        assert!(!berths.is_empty(), "berths must not be empty");
        match stance {
            BerthStance::Still => {}
            BerthStance::Circling { speed, .. } | BerthStance::Roaming { speed, .. } => {
                assert!(
                    speed > FixedU64::ZERO,
                    "a berth-to-berth speed must be greater than 0"
                );
            }
            BerthStance::Orbit { radius, period } => {
                assert!(
                    radius > FixedU64::ZERO,
                    "an orbit radius must be greater than 0"
                );
                assert!(period > 0, "an orbit period must be greater than 0");
            }
        }
        Self { berths, stance }
    }

    /// The berth group of the job the worker sits in.
    #[inline]
    pub fn berths(&self) -> &str {
        &self.berths
    }

    /// How the worker sits in its berth over time.
    #[inline]
    pub fn stance(&self) -> BerthStance {
        self.stance
    }
}

/// Where a worker stands while it attends a job, and how many may attend it.
///
/// Every worker on a job contributes its own rate and pays its own way, so
/// massing workers buys speed without buying it cheaper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkPresence {
    /// Taken off the map for the duration, so the work site is left clear.
    Hidden {
        /// How many workers may work one job at once.
        crew: CrewLimit,
    },
    /// Standing beside the work, where it can be shot at.
    Present {
        /// How many workers may work one job at once.
        crew: CrewLimit,
    },
    /// Sitting in one of the job's berths, where it can be shot at, holding no
    /// cells: nothing is blocked by it and nothing pushes it. As many workers at
    /// a time as the berth group has slots, each contributing its own rate and
    /// paying its own way.
    Attached(Attachment),
}

impl WorkPresence {
    /// How the worker attaches to its job, or `None` when it does not.
    #[inline]
    pub fn attachment(&self) -> Option<&Attachment> {
        match self {
            WorkPresence::Attached(attachment) => Some(attachment),
            WorkPresence::Hidden { .. } | WorkPresence::Present { .. } => None,
        }
    }

    /// What decides how many workers of this presence may work one job at once.
    #[inline]
    pub fn crewing(&self) -> Crewing {
        match self {
            WorkPresence::Hidden { crew } | WorkPresence::Present { crew } => {
                Crewing::Counted(*crew)
            }
            WorkPresence::Attached(_) => Crewing::Seated,
        }
    }
}
