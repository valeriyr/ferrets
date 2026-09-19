//! Engine-reserved classification tag names.

/// The tag marking an entity as a building. Pre-registered, so content may apply
/// it without declaring it.
pub const BUILDING: &str = "building";

/// The tag marking an entity as remains: something left lying where it was put
/// rather than standing up.
///
/// An entity wearing it belongs to nobody, takes no orders, is picked out by
/// nothing, and lies there for its `lifetime` before it is gone.
pub const REMAINS: &str = "remains";
