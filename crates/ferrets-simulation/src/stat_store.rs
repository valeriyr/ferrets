//! Index-transparent runtime store the stat folds run in.

use ferrets_content::stats::{self, ModifierOp};
use ferrets_math::{FixedI64, FixedU64};

/// One stat's base value and its effective value after unconditional modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatCell {
    /// The authored value, before any modifier folds in.
    base: FixedU64,
    /// The value after unconditional modifiers; equals `base` until the modifier
    /// pipeline recomputes it.
    effective: FixedU64,
}

impl StatCell {
    #[inline]
    fn new(base: FixedU64) -> Self {
        Self {
            base,
            effective: base,
        }
    }
}

/// Index-transparent store of stat cells — the one implementation every stat
/// group computes on.
///
/// It knows registration indices, values, and the floors handed to it, never the
/// typed ids: the wrappers around it are the gates that keep the groups apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StatStore {
    /// Indexed by registration index; `None` for stats the owner does not have.
    cells: Vec<Option<StatCell>>,
}

impl StatStore {
    /// Sets a stat's base value, resetting its effective value to match, and
    /// growing the store as needed.
    pub(crate) fn set_base(&mut self, index: usize, base: FixedU64) {
        if index >= self.cells.len() {
            self.cells.resize(index + 1, None);
        }
        self.cells[index] = Some(StatCell::new(base));
    }

    /// The base value at `index`, or `None` if the owner does not have it.
    pub(crate) fn base(&self, index: usize) -> Option<FixedU64> {
        self.cell(index).map(|c| c.base)
    }

    /// The effective value at `index`, or `None` if the owner does not have it.
    pub(crate) fn effective(&self, index: usize) -> Option<FixedU64> {
        self.cell(index).map(|c| c.effective)
    }

    /// `true` if the owner has the stat at `index`.
    pub(crate) fn has(&self, index: usize) -> bool {
        self.cell(index).is_some()
    }

    /// The present cell at `index`, if any.
    #[inline]
    fn cell(&self, index: usize) -> Option<StatCell> {
        self.cells.get(index).copied().flatten()
    }

    /// Recomputes every present stat's effective value from its base and the
    /// `(index, op, magnitude)` entries targeting it, holding each result at the
    /// floor `floor_of` names for its index. Entries for stats the owner does
    /// not have are ignored. The fold is order-independent.
    ///
    /// Callers hand in their own group's arm already reduced to indices — the
    /// store never sees the other arm, or any typed id at all.
    pub(crate) fn recompute(
        &mut self,
        modifiers: &[(usize, ModifierOp, FixedI64)],
        floor_of: impl Fn(usize) -> FixedU64,
    ) {
        for (index, cell) in self.cells.iter_mut().enumerate() {
            let Some(cell) = cell else { continue };
            let targeting = modifiers
                .iter()
                .filter(|(target, _, _)| *target == index)
                .map(|&(_, op, magnitude)| (op, magnitude));
            cell.effective = stats::fold(cell.base, targeting).max(floor_of(index));
        }
    }
}
