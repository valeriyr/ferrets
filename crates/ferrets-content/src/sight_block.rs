//! What blocks sight: how far a blocker's top rises above where it stands,
//! and whose sight it blocks.

use crate::relation::Relations;

/// A blocker of sight standing on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SightBlock {
    /// Levels its top rises above the level it stands at.
    height: u32,
    /// Whose sight it blocks, judged from whoever it belongs to.
    of: Relations,
}

impl SightBlock {
    /// Creates a new `SightBlock` with the given data.
    pub fn new(height: u32, of: Relations) -> Self {
        Self { height, of }
    }

    /// Levels its top rises above the level it stands at.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whose sight it blocks, judged from whoever it belongs to.
    #[inline]
    pub fn of(&self) -> Relations {
        self.of
    }
}
