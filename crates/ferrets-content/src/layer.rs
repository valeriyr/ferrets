//! The layer vocabulary: where a layer is on the vertical scale and the
//! viewers its relief blocks the sight of.

use std::fmt::Display;

use ferrets_nav_grid::layer_mask::LayerMask;

/// One layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerDef {
    /// The levels the layer is at.
    level: LayerLevel,
    /// The layers whose viewers its levels block the sight of.
    relief_blocks: LayerMask,
}

impl LayerDef {
    /// Creates a new `LayerDef` with the given data.
    pub fn new(level: LayerLevel, relief_blocks: LayerMask) -> Self {
        Self {
            level,
            relief_blocks,
        }
    }

    /// The levels the layer is at.
    #[inline]
    pub fn level(&self) -> LayerLevel {
        self.level
    }

    /// The layers whose viewers its levels block the sight of.
    #[inline]
    pub fn relief_blocks(&self) -> LayerMask {
        self.relief_blocks
    }
}

/// The levels a layer is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerLevel {
    /// One level, the layer flat at it everywhere.
    At(u32),
    /// Every level from `from` to `to`, both included; the layer's level at
    /// each cell is one of them.
    Range {
        /// The lowest level.
        from: u32,
        /// The highest level.
        to: u32,
    },
}

impl LayerLevel {
    /// Whether `level` is one of the layer's levels.
    pub fn holds(self, level: u32) -> bool {
        match self {
            Self::At(at) => at == level,
            Self::Range { from, to } => (from..=to).contains(&level),
        }
    }

    /// Whether every level of `self` is below every level of `other`.
    pub fn below(self, other: Self) -> bool {
        self.span().1 < other.span().0
    }

    /// Whether the two share a level.
    pub fn meets(self, other: Self) -> bool {
        let (low, high) = self.span();
        let (other_low, other_high) = other.span();
        low <= other_high && other_low <= high
    }

    /// The lowest and the highest of the levels.
    fn span(self) -> (u32, u32) {
        match self {
            Self::At(at) => (at, at),
            Self::Range { from, to } => (from, to),
        }
    }
}

impl Display for LayerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::At(at) => write!(f, "{at}"),
            Self::Range { from, to } => write!(f, "{from}..={to}"),
        }
    }
}
