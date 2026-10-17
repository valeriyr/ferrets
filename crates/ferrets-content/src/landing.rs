//! Which cells a body may be set down on, by what the setting player sees.

/// Which cells a body may be set down on, judged for the sight the acting
/// player's seat declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Any cell, seen or not.
    Anywhere,
    /// A cell the team has seen at least once.
    Explored,
    /// A cell the team sees now.
    Sighted,
}
