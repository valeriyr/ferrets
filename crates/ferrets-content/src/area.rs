//! Which cells around a shape something takes in.

use ferrets_math::FixedU64;

use crate::quantity::Quantity;

/// Which cells around the shape it is measured from something takes in, as
/// content declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// Only what the shape covers.
    Inside,
    /// What lies within this many cells of the shape: the gap between the
    /// two, `0` taking in what touches it.
    Around(Quantity),
}

impl Area {
    /// Whether this area is known, without reading a stat, to reach farther
    /// than `bound`: anything around the shape reaches farther than its
    /// inside, and two constant gaps compare; a gap read from a stat is not
    /// known.
    pub fn known_beyond(self, bound: Area) -> bool {
        match (self, bound) {
            (Area::Around(_), Area::Inside) => true,
            (Area::Around(Quantity::Constant(cells)), Area::Around(Quantity::Constant(limit))) => {
                cells > limit
            }
            (Area::Inside, _) | (Area::Around(_), Area::Around(_)) => false,
        }
    }
}

/// An [`Area`] whose quantity has been read: a constant as declared, a stat
/// as the entity carrying it held it then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedArea {
    /// Only what the shape covers.
    Inside,
    /// What lies within this many cells of the shape.
    Around(FixedU64),
}

impl ResolvedArea {
    /// This area reaching `width` cells further: from `Inside`, `width` of
    /// the shape, so a width of `0` takes in what touches it.
    pub fn widened(self, width: FixedU64) -> Self {
        match self {
            ResolvedArea::Inside => ResolvedArea::Around(width),
            ResolvedArea::Around(cells) => ResolvedArea::Around(cells.saturating_add(width)),
        }
    }

    /// How many whole cells this area reaches out, measured from a
    /// footprint's own cells: `Inside` is `0`, and `Around` its cells
    /// truncated.
    pub fn reach(self) -> u32 {
        match self {
            ResolvedArea::Inside => 0,
            ResolvedArea::Around(cells) => cells.to_num::<u32>(),
        }
    }
}
