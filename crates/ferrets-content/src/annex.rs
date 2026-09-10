//! Annexes: buildings bound to a dock of the building beside them.
//!
//! The two sides are declared apart. A primary offers docks — where an annex
//! stands on it, and which annexes it takes. An annex declares what it does
//! with no primary docked, and who may dock with it.

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;

/// One dock a type offers: the spot beside it an annex stands on, and which
/// annexes it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockDef {
    /// Where the annex's anchor sits, in cells from this footprint's own
    /// origin.
    at: CellPos,
    /// The annex types the dock takes, by registered name.
    accepts: Vec<String>,
}

impl DockDef {
    /// Creates a new `DockDef` with the given data.
    ///
    /// Panics if `accepts` is empty or holds an empty name.
    pub fn new(at: CellPos, accepts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let accepts: Vec<String> = accepts.into_iter().map(Into::into).collect();

        assert!(!accepts.is_empty(), "a dock must accept at least one type");
        assert!(
            accepts.iter().all(|name| !name.is_empty()),
            "accepted annex names must not be empty"
        );

        Self { at, accepts }
    }

    /// Where the annex's anchor sits, in cells from the footprint's own origin.
    #[inline]
    pub fn at(&self) -> CellPos {
        self.at
    }

    /// Whether the dock takes annexes of `type_name`.
    pub fn accepts(&self, type_name: &str) -> bool {
        self.accepts.iter().any(|name| name == type_name)
    }

    /// The annex types the dock takes.
    pub fn accepted_types(&self) -> impl Iterator<Item = &str> {
        self.accepts.iter().map(String::as_str)
    }
}

/// Whether an annex with no primary docked carries out its type's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnexWork {
    /// It stands switched off: it starts no order, and production already
    /// under way holds where it stood until a primary docks again.
    Idles,
    /// It works as it does docked.
    Works,
}

/// What time does to an annex with no primary docked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnexLife {
    /// It stands until something brings it down.
    Endures,
    /// It loses health every tick, and dies when the pool runs out.
    Fades {
        /// Health lost per tick.
        per_tick: FixedU64,
    },
}

/// What becomes of an annex with no primary docked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AloneConduct {
    /// It comes down the tick it loses its primary.
    Razed,
    /// It stands on its own, on these terms.
    Standing {
        /// Whether it carries out its type's work.
        work: AnnexWork,
        /// What time does to it.
        life: AnnexLife,
    },
}

/// Who may dock with an annex, and what docking does to whose it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnexClaim {
    /// Only its owner's primaries dock with it.
    Bound,
    /// Its owner's primaries and its allies' dock with it, and whose it is
    /// never changes.
    Allied,
    /// Anyone's primary docks with it, and it becomes that primary's owner's.
    Seized,
}

/// Content-defined annex properties for an entity type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnexDef {
    /// What it does with no primary docked.
    alone: AloneConduct,
    /// Who may dock with it, and what that does to whose it is.
    claim: AnnexClaim,
}

impl AnnexDef {
    /// Creates a new `AnnexDef` with the given data.
    ///
    /// Panics if a fading life loses no health per tick.
    pub fn new(alone: AloneConduct, claim: AnnexClaim) -> Self {
        match alone {
            AloneConduct::Standing {
                life: AnnexLife::Fades { per_tick },
                ..
            } => assert!(
                per_tick > FixedU64::ZERO,
                "a fading annex must lose health every tick"
            ),
            AloneConduct::Razed
            | AloneConduct::Standing {
                life: AnnexLife::Endures,
                ..
            } => {}
        }

        Self { alone, claim }
    }

    /// What it does with no primary docked.
    #[inline]
    pub fn alone(&self) -> AloneConduct {
        self.alone
    }

    /// Who may dock with it, and what that does to whose it is.
    #[inline]
    pub fn claim(&self) -> AnnexClaim {
        self.claim
    }
}
