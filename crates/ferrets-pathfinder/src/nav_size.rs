//! Navigation grid footprint size.

/// Footprint of an entity on the navigation grid, in whole cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NavSize {
    pub width: u32,
    pub height: u32,
}

impl NavSize {
    /// A 1×1 footprint — the default for most units.
    pub const ONE: Self = Self::new(1, 1);

    #[inline]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}
