//! What moving a pool's maximum does to the current value under it.

/// How the current value of a pool follows when something moves its maximum
/// from `old` to `new`; each result is held under `new`. A maximum whose stat
/// may fall to zero empties its pool there, and what returns follows the
/// shift from zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolShift {
    /// The current value keeps its share of the maximum:
    /// `current × new / old`; from a zero maximum, nothing.
    Share,
    /// The current value moves by the same amount as the maximum:
    /// `current + (new − old)`; from a zero maximum, the whole of `new`.
    Difference,
    /// The current value stays where it is, only held under the maximum:
    /// `min(current, new)`; from a zero maximum, nothing.
    Clamp,
}
