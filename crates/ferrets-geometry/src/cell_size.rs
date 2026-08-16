//! Grid footprint size.

/// The size of an entity's footprint on the cell grid, in whole cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

impl CellSize {
    /// The 1×1 footprint size — the default for most units.
    pub const ONE: Self = Self::new(1, 1);

    #[inline]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}
