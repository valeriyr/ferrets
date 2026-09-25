//! Content-defined repair capability: what an entity mends, and on what terms.

use ferrets_math::FixedU64;

use crate::{kinds::Kinds, price::Price, work::WorkPresence};

/// How fast the work goes, before the repairer's `repair_speed` stat scales it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairRate {
    /// Paced against the target's own production time and `repair_ratio`, so
    /// restoring a share of a pool takes that share of the time it took to produce
    /// one. A target nothing produces cannot be worked on at this rate.
    Production,
    /// A flat number of health points per tick, the same for every target.
    /// Where a unit's price says nothing about how long it takes to patch up.
    PerTick(FixedU64),
}

/// What a repairer pays for the work it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairCost {
    /// Nothing.
    Free,
    /// A share of the target's own price, in proportion to the health restored,
    /// scaled by the repairer's `repair_cost_factor` stat. Mending a third of a
    /// pool costs a third of the price, so the bill does not depend on how many
    /// workers attend.
    ProRata,
    /// A fixed amount each tick, charged for every worker on the job.
    PerTick(Price),
    /// The repairer's own energy, per point of health restored. Spent from its pool
    /// rather than the owner's stockpile, so the limit is the worker's stamina and
    /// its regeneration rather than the economy.
    Energy(FixedU64),
}

/// Content-defined repair capability: the tags an entity mends and the terms it
/// mends them on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairerDef {
    /// What this entity will mend.
    repairs: Kinds,
    /// What sets the pace of the work.
    rate: RepairRate,
    /// Where the worker stands while it works.
    presence: WorkPresence,
    /// Whether the entity may mend itself.
    self_repair: bool,
    /// What the work costs.
    cost: RepairCost,
    /// Ticks the worker waits, unable to pay, before abandoning the job. `None`
    /// means it waits indefinitely.
    patience: Option<u32>,
}

impl RepairerDef {
    /// Creates a new `RepairerDef` with the given data.
    ///
    /// Panics if the rate is a non-positive flat amount.
    pub fn new(
        repairs: Kinds,
        rate: RepairRate,
        presence: WorkPresence,
        self_repair: bool,
        cost: RepairCost,
        patience: Option<u32>,
    ) -> Self {
        if let RepairRate::PerTick(health) = rate {
            assert!(
                health > FixedU64::ZERO,
                "a flat repair rate must be positive"
            );
        }

        Self {
            repairs,
            rate,
            presence,
            self_repair,
            cost,
            patience,
        }
    }

    /// What sets the pace of the work.
    #[inline]
    pub fn rate(&self) -> RepairRate {
        self.rate
    }

    /// What this entity will mend.
    #[inline]
    pub fn repairs(&self) -> &Kinds {
        &self.repairs
    }

    /// Where the worker stands while it works.
    #[inline]
    pub fn presence(&self) -> &WorkPresence {
        &self.presence
    }

    /// Whether the entity may mend itself.
    #[inline]
    pub fn self_repair(&self) -> bool {
        self.self_repair
    }

    /// What the work costs.
    #[inline]
    pub fn cost(&self) -> &RepairCost {
        &self.cost
    }

    /// Ticks the worker waits, unable to pay, before abandoning the job.
    #[inline]
    pub fn patience(&self) -> Option<u32> {
        self.patience
    }
}
