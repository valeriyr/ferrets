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

/// Where a worker stands while it attends a job, and whether others may join it.
///
/// Declared per capability rather than per entity: one worker can reasonably
/// disappear into a job it does alone and stand out in the open beside another its
/// fellows crowd around, so a single setting per entity could not express both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkPresence {
    /// Taken off the map for the duration, so the work site is left clear. One
    /// worker at a time.
    Hidden,
    /// Standing beside the work, where it can be shot at. One worker at a time.
    Present,
    /// Standing beside the work, with any number of others alongside it. Each one
    /// contributes its own rate and pays its own way, so massing workers buys speed
    /// without buying it cheaper.
    PresentStacking,
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
            WorkPresence::Hidden | WorkPresence::Present | WorkPresence::PresentStacking => None,
        }
    }

    /// Whether several workers may share one job.
    #[inline]
    pub fn stacks(&self) -> bool {
        matches!(
            self,
            WorkPresence::PresentStacking | WorkPresence::Attached(_)
        )
    }
}
