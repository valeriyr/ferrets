//! Content-defined breeding: what an entity type bears on a timer and on what
//! terms, and how a bred type is kept by its breeder.

use crate::{period::Period, work::Attachment};

/// What becomes of the broodlings when the breeder dies, or lands a form
/// that cannot hold them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanFate {
    /// They die with it.
    Perish,
    /// They are set down beside it and stay, with no tie left.
    Linger(Lingering),
}

/// How a broodling set down by its breeder lingers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lingering {
    /// Where it was set down, forgotten by the breeder.
    Stay,
    /// Where it was set down, until a breeder of its player that breeds its
    /// type and has a seat free takes it in: one within `distance` cells of
    /// the breeder's footprint, standing idle and untied.
    Reseat {
        /// How far from a breeder's footprint a set-down broodling may stand
        /// and still be taken in, in cells.
        distance: u32,
    },
}

/// Content-defined broodling properties, held by a type something breeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroodlingDef {
    /// How instances sit in their breeder's berths: the group and the stance.
    attachment: Attachment,
}

impl BroodlingDef {
    /// Creates a new `BroodlingDef` with the given data.
    pub fn new(attachment: Attachment) -> Self {
        Self { attachment }
    }

    /// How instances sit in their breeder's berths.
    #[inline]
    pub fn attachment(&self) -> &Attachment {
        &self.attachment
    }
}

/// What an entity type breeds, and on what terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreederDef {
    /// The type bred, by registered name.
    breeds: String,
    /// Ticks between births while below the limit.
    period: Period,
    /// How many broodlings at once.
    limit: usize,
    /// How many broodlings the form opens with, at least: a breeder raised in
    /// it, or landing a change of form in it, is owed the births that bring
    /// its brood up to this many. Never more than the limit.
    initial: usize,
    /// What becomes of the broodlings when the breeder dies, or lands a form
    /// that cannot hold them.
    orphans: OrphanFate,
}

impl BreederDef {
    /// Creates a new `BreederDef` with the given data.
    ///
    /// Panics if `breeds` is empty, `limit` is `0`, or `initial` exceeds
    /// `limit`.
    pub fn new(
        breeds: impl Into<String>,
        period: Period,
        limit: usize,
        initial: usize,
        orphans: OrphanFate,
    ) -> Self {
        let breeds = breeds.into();
        assert!(!breeds.is_empty(), "breeds must not be empty");
        assert!(limit > 0, "a brood limit must admit at least one broodling");
        assert!(
            initial <= limit,
            "a brood cannot owe more broodlings than its limit admits"
        );
        Self {
            breeds,
            period,
            limit,
            initial,
            orphans,
        }
    }

    /// The type bred, by registered name.
    #[inline]
    pub fn breeds(&self) -> &str {
        &self.breeds
    }

    /// Ticks between births while below the limit.
    #[inline]
    pub fn period(&self) -> Period {
        self.period
    }

    /// How many broodlings at once.
    #[inline]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// How many broodlings the form opens with, at least.
    #[inline]
    pub fn initial(&self) -> usize {
        self.initial
    }

    /// What becomes of the broodlings when the breeder dies, or lands a form
    /// that cannot hold them.
    #[inline]
    pub fn orphans(&self) -> OrphanFate {
        self.orphans
    }
}
