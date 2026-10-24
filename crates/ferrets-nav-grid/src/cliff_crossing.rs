//! How a mover crosses a cliff.

/// How a mover crosses a cliff: an edge between neighboring cells one or more
/// levels apart with no ramp joining them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CliffCrossing {
    /// It changes level only over a ramp.
    Ramp,
    /// It also steps up or down a cliff of one level.
    Cross,
}
