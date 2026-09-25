//! Content-defined in-place transitions: what an entity can become, and on
//! what terms.

use crate::{cost::Cost, quantity::Quantity, requirement::Requirement};

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

/// What becomes of the entity when a transition is interrupted — called off,
/// flushed, or landing on ground that no longer takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphInterrupted {
    /// It reverts to the origin form, standing where the change was under way.
    Reverts,
    /// It dies.
    Dies,
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
    /// The form worn while the transition runs, by registered name: entered
    /// when the transition starts and left when it lands. `None` keeps the
    /// origin form for the duration.
    via: Option<String>,
    /// How long the transition takes.
    time: Quantity,
    /// When the destination footprint is secured.
    placement: MorphPlacement,
    /// Whether the transition can be called off once under way.
    cancel: MorphCancel,
    /// What becomes of the entity when the transition is interrupted.
    interrupted: MorphInterrupted,
    /// What the transition is for, as the statistics count it.
    reason: MorphReason,
    /// What starting the transition costs, drawn when it starts. Every arm is
    /// checked before any is paid. Empty means free.
    costs: Vec<Cost>,
    /// Requirements gating the transition, read the same way as a type's own
    /// [`requires`](crate::entity_type_def::EntityTypeDef::requires) list.
    /// Empty means always available.
    requires: Vec<Requirement>,
}

impl MorphTransition {
    /// Creates a new `MorphTransition` with the given data.
    ///
    /// Panics if `into` or `via` is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        into: impl Into<String>,
        via: Option<&str>,
        time: Quantity,
        placement: MorphPlacement,
        cancel: MorphCancel,
        interrupted: MorphInterrupted,
        reason: MorphReason,
        costs: Vec<Cost>,
        requires: impl IntoIterator<Item = Requirement>,
    ) -> Self {
        let into = into.into();
        assert!(!into.is_empty(), "into must not be empty");
        let via = via.map(str::to_string);
        assert!(
            via.as_ref().is_none_or(|via| !via.is_empty()),
            "via must not be empty"
        );
        let requires: Vec<Requirement> = requires.into_iter().collect();

        Self {
            into,
            via,
            time,
            placement,
            cancel,
            interrupted,
            reason,
            costs,
            requires,
        }
    }

    /// The destination type's registered name.
    #[inline]
    pub fn into_type(&self) -> &str {
        &self.into
    }

    /// The form worn while the transition runs, if any.
    #[inline]
    pub fn via_type(&self) -> Option<&str> {
        self.via.as_deref()
    }

    /// How long the transition takes.
    #[inline]
    pub fn time(&self) -> Quantity {
        self.time
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

    /// What becomes of the entity when the transition is interrupted.
    #[inline]
    pub fn interrupted(&self) -> MorphInterrupted {
        self.interrupted
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
