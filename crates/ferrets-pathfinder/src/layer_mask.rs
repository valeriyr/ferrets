//! A navigation grid layer mask — a bitmask of one or more navigation layers.

use std::{
    fmt::Display,
    ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Deref, Not},
};

use crate::layer_id::LayerId;

/// A bitmask of one or more navigation layers.
///
/// Each set bit corresponds to one [`LayerId`]. Build a mask by combining
/// [`LayerId`]s with `|`, then pass it to methods that accept multi-layer queries.
///
/// A single [`LayerId`] converts to a `LayerMask` via [`From`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayerMask(u32);

impl LayerMask {
    /// An empty mask with no layers set.
    pub const EMPTY: Self = Self(0);
}

impl From<u32> for LayerMask {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<LayerId> for LayerMask {
    fn from(id: LayerId) -> Self {
        Self(*id)
    }
}

impl Display for LayerMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#b}", self.0)
    }
}

impl Deref for LayerMask {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Not for LayerMask {
    type Output = Self;

    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl BitAnd for LayerMask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAnd<LayerId> for LayerMask {
    type Output = Self;

    fn bitand(self, rhs: LayerId) -> Self {
        Self(self.0 & *rhs)
    }
}

impl BitAndAssign for LayerMask {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl BitOr for LayerMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<LayerId> for LayerMask {
    type Output = Self;

    fn bitor(self, rhs: LayerId) -> Self {
        Self(self.0 | *rhs)
    }
}

impl BitOrAssign for LayerMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<LayerId> for LayerMask {
    fn bitor_assign(&mut self, rhs: LayerId) {
        self.0 |= *rhs;
    }
}

impl PartialEq<u32> for LayerMask {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}
