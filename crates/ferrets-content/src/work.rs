//! Shared vocabulary for how a worker attends a job it takes time to finish.

/// Where a worker stands while it works, and whether others may join it.
///
/// Declared per capability rather than per entity: one worker can reasonably
/// disappear into a job it does alone and stand out in the open beside another its
/// fellows crowd around, so a single setting per entity could not express both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl WorkPresence {
    /// Whether the worker leaves the map while it works.
    #[inline]
    pub fn is_hidden(self) -> bool {
        matches!(self, WorkPresence::Hidden)
    }

    /// Whether several workers may share one job.
    #[inline]
    pub fn stacks(self) -> bool {
        matches!(self, WorkPresence::PresentStacking)
    }
}
