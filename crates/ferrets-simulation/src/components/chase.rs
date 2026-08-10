//! Chase progress shared by every order that walks toward something.

use ferrets_geometry::cell_pos::CellPos;

/// The last chase round, recorded when its walk started. Rounds that begin
/// from the same chaser cell against the same destination cell made no
/// progress; a run of them means the destination cannot be reached and the
/// chase gives up (see [`ChaseRound::exhausted`]).
///
/// Cells, not positions: a continuous mover's position wobbles by bits under
/// pushing, so exact positions would never repeat and a chase against an
/// unreachable destination would re-walk forever instead of giving up. A few
/// identical rounds rather than one: a crowd can bounce a chaser back to the
/// cell it walked from — a passer-by, a contested work spot — and one bounce
/// says nothing about reachability, while a wall says the same thing every
/// round.
pub type ChaseState = Option<ChaseRound>;

/// One recorded chase round: where the chaser stood, what it walked toward,
/// and how many rounds in a row started exactly there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChaseRound {
    /// The cell under the chaser's body center when the round started.
    pub own: CellPos,
    /// The destination cell the round walked toward.
    pub destination: CellPos,
    /// Consecutive rounds that started from this same pair.
    pub repeats: u32,
}

impl ChaseRound {
    /// How many identical rounds a chase tolerates before reading the
    /// destination as unreachable.
    const PATIENCE: u32 = 3;

    /// Whether this many identical rounds means the chase should give up.
    pub fn exhausted(&self) -> bool {
        self.repeats >= Self::PATIENCE
    }
}
