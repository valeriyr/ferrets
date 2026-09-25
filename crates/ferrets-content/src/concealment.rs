//! Whether a type's instances are seen wherever a side's sight reaches, or
//! only where its detection reaches as well.

/// How an entity type stands toward a side that is not its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Concealment {
    /// Seen wherever the side's sight reaches any cell it stands on.
    Exposed,
    /// Seen only where the side's detection reaches one of those cells as well.
    Concealed,
}
