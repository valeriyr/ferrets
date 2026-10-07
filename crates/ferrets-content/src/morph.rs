//! Content-defined in-place transitions: what an entity can become, and on
//! what terms.

use std::collections::BTreeMap;

use crate::{
    cost::Cost, pool_def::PoolId, pool_shift::PoolShift, quantity::Quantity,
    requirement::Requirement,
};

/// When a transition secures the ground its destination form stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphPlacement {
    /// The destination footprint is claimed when the transition starts: a
    /// start is refused unless the footprint fits, and completion is then
    /// guaranteed.
    Reserve,
    /// The destination footprint is checked only at completion: the
    /// transition always starts, and fizzles if the footprint no longer fits.
    Revalidate,
    /// The destination footprint is set down at completion on the nearest
    /// free cells to where the entity stands: the transition always starts,
    /// and fizzles only when nothing within the placement search radius fits.
    /// Only a form that can move lands this way; the registry refuses it for
    /// a static one.
    Nearby,
}

/// Whether a transition under way can be called off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphCancel {
    /// Cannot be called off: a cancel is refused, and a forced flush loses
    /// whatever was paid.
    Committed,
    /// Can be called off, but whatever was paid stays paid.
    Forfeit,
    /// Gives back what it cost when its owner calls it off, and keeps it when
    /// the change is taken away.
    Refundable,
}

/// How a landing fills the pools the form it lands on carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolCarry {
    /// Each pool moves to the new maximum as the shift says, from the maximum
    /// it stood under before the landing.
    Shift(PoolShift),
    /// Each pool comes out full.
    Full,
    /// Each pool starts at the initial the form it comes into declares.
    Initial,
}

/// How a change that reverts out of its interim form fills the origin's
/// pools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertCarry {
    /// Each pool comes back to what it held when the change started.
    Restore,
    /// Each pool carries from the interim form's values, as on any landing.
    Carry(PoolCarry),
}

/// What becomes of the entity when a direct change is interrupted — called
/// off, flushed, or landing on ground that no longer takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphInterrupted {
    /// It stays in the origin form, its pools as they are.
    Reverts,
    /// It dies.
    Dies,
}

/// What becomes of the entity when a change through an interim form is
/// interrupted — called off, flushed, or landing on ground that no longer
/// takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViaInterrupted {
    /// It goes back to the origin form, standing where the change was under
    /// way, each pool named filled as its carry says and every other one
    /// keeping its value, held under the origin's maximum.
    Reverts(BTreeMap<PoolId, RevertCarry>),
    /// It dies.
    Dies,
}

impl ViaInterrupted {
    /// A change that reverts on interruption, each pool in `carries` filled as
    /// its carry says.
    pub fn reverts(carries: impl IntoIterator<Item = (PoolId, RevertCarry)>) -> Self {
        Self::Reverts(carries.into_iter().collect())
    }
}

/// How a change gets from its origin form to its destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MorphCourse {
    /// The form changes at the landing; until then the origin stands as it
    /// is.
    Direct {
        /// What becomes of the entity when the change is interrupted.
        interrupted: MorphInterrupted,
    },
    /// An interim form is worn while the change runs: entered when it starts
    /// and left when it lands.
    Via {
        /// The interim form, by registered name.
        form: String,
        /// How the landing on the interim form fills each pool it names; a
        /// pool it does not name keeps its value, held under the new maximum.
        enter_pool_carry: BTreeMap<PoolId, PoolCarry>,
        /// What becomes of the entity when the change is interrupted.
        interrupted: ViaInterrupted,
    },
}

impl MorphCourse {
    /// A change whose form changes at the landing.
    pub fn direct(interrupted: MorphInterrupted) -> Self {
        Self::Direct { interrupted }
    }

    /// A change that wears `form` while it runs, entering it with each pool in
    /// `enter_pool_carry` filled as its carry says.
    pub fn via(
        form: impl Into<String>,
        enter_pool_carry: impl IntoIterator<Item = (PoolId, PoolCarry)>,
        interrupted: ViaInterrupted,
    ) -> Self {
        Self::Via {
            form: form.into(),
            enter_pool_carry: enter_pool_carry.into_iter().collect(),
            interrupted,
        }
    }
}

/// What a transition is for, as the statistics count it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphReason {
    /// Producing the destination type: a landing counts as producing one.
    Production,
    /// Changing what the entity is: a landing counts for nothing.
    Change,
}

/// One transition an entity type offers: what it becomes, and on what terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MorphTransition {
    /// The destination type, by registered name. Named rather than handled
    /// because transitions may be circular: two forms can each name the
    /// other, so no registration order resolves both to a handle.
    into: String,
    /// How the transition gets from the origin form to the destination.
    course: MorphCourse,
    /// How long the transition takes.
    time: Quantity,
    /// When the destination footprint is secured.
    placement: MorphPlacement,
    /// Whether the transition can be called off once under way.
    cancel: MorphCancel,
    /// What the transition is for, as the statistics count it.
    reason: MorphReason,
    /// What starting the transition costs, drawn when it starts. Every arm is
    /// checked before any is paid. Empty means free.
    costs: Vec<Cost>,
    /// Requirements gating the transition, read the same way as a type's own
    /// [`requires`](crate::entity_type_def::EntityTypeDef::requires) list.
    /// Empty means always available.
    requires: Vec<Requirement>,
    /// How the landing on the destination fills each pool it names; a pool it
    /// does not name keeps its value, held under the new maximum.
    land_pool_carry: BTreeMap<PoolId, PoolCarry>,
}

impl MorphTransition {
    /// Creates a new `MorphTransition` with the given data.
    ///
    /// Panics if `into` or the interim form's name is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        into: impl Into<String>,
        course: MorphCourse,
        time: Quantity,
        placement: MorphPlacement,
        cancel: MorphCancel,
        reason: MorphReason,
        costs: Vec<Cost>,
        requires: impl IntoIterator<Item = Requirement>,
        land_pool_carry: impl IntoIterator<Item = (PoolId, PoolCarry)>,
    ) -> Self {
        let into = into.into();
        assert!(!into.is_empty(), "into must not be empty");
        match &course {
            MorphCourse::Direct { .. } => {}
            MorphCourse::Via { form, .. } => {
                assert!(!form.is_empty(), "via must not be empty");
            }
        }
        let requires: Vec<Requirement> = requires.into_iter().collect();

        Self {
            into,
            course,
            time,
            placement,
            cancel,
            reason,
            costs,
            requires,
            land_pool_carry: land_pool_carry.into_iter().collect(),
        }
    }

    /// The destination type's registered name.
    #[inline]
    pub fn into_type(&self) -> &str {
        &self.into
    }

    /// How the transition gets from the origin form to the destination.
    #[inline]
    pub fn course(&self) -> &MorphCourse {
        &self.course
    }

    /// The form worn while the transition runs, if any.
    #[inline]
    pub fn via_type(&self) -> Option<&str> {
        match &self.course {
            MorphCourse::Direct { .. } => None,
            MorphCourse::Via { form, .. } => Some(form),
        }
    }

    /// How long the transition takes.
    #[inline]
    pub fn time(&self) -> Quantity {
        self.time
    }

    /// How the landing on the destination fills each pool it names.
    #[inline]
    pub fn land_pool_carry(&self) -> impl Iterator<Item = (PoolId, PoolCarry)> + '_ {
        self.land_pool_carry
            .iter()
            .map(|(&pool, &carry)| (pool, carry))
    }

    /// When the destination footprint is secured.
    #[inline]
    pub fn placement(&self) -> MorphPlacement {
        self.placement
    }

    /// Whether the transition can be called off once under way.
    #[inline]
    pub fn cancel(&self) -> MorphCancel {
        self.cancel
    }

    /// What the transition is for, as the statistics count it.
    #[inline]
    pub fn reason(&self) -> MorphReason {
        self.reason
    }

    /// What starting the transition costs.
    #[inline]
    pub fn costs(&self) -> &[Cost] {
        &self.costs
    }

    /// Requirements gating the transition.
    #[inline]
    pub fn requires(&self) -> &[Requirement] {
        &self.requires
    }
}
