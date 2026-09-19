//! Whose something must be — the one way every capability says it.

/// Whose an entity, a coverage or a claim must be, judged from the point of
/// view of whoever is asking.
///
/// A rule that names one of these is read against the asker's own owner: a
/// weapon's owner, a caster's, the owner of the footprint a placement is being
/// tested for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affiliation {
    /// The asker's own, and nobody else's.
    Own,
    /// The asker's own or an ally's.
    Allied,
    /// A player hostile to the asker's.
    Enemy,
    /// Anyone at all, the unowned included.
    Anyone,
}
