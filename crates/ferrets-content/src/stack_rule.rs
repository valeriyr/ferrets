//! The stacking vocabulary shared by every buff kind.

/// What happens when a buff is applied to a carrier that already holds one of
/// the same id. There is no engine default — content declares the rule per
/// buff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackRule {
    /// Keep the single instance and reset its remaining duration.
    Refresh,
    /// Add a stack (its modifiers apply once more), up to `cap`, and refresh the
    /// duration.
    StackToCap(u32),
    /// Keep the existing instance unchanged; drop the new application.
    Ignore,
}
