//! Content-defined in-place transitions: what an entity can become, and on
//! what terms.

use crate::{entity_stats::EntityStatId, skills::EntityCastCost};

/// How long a transition takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphTime {
    /// A fixed number of ticks. Zero completes in the same tick it starts.
    Constant(u32),
    /// Read from the changing entity's effective stats each tick, so the
    /// modifier pipeline can move it while the change is under way.
    Stat(EntityStatId),
}

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
}

/// Whether a transition under way can be called off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphCancel {
    /// Cannot be called off: a cancel is refused, and a forced flush loses
    /// whatever was paid.
    Committed,
    /// Can be called off, but whatever was paid stays paid.
    Forfeit,
    /// Can be called off with a full refund of whatever was paid.
    Refundable,
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
    time: MorphTime,
    /// When the destination footprint is secured.
    placement: MorphPlacement,
    /// Whether the transition can be called off once under way.
    cancel: MorphCancel,
    /// What starting the transition costs, drawn when it starts. Every arm is
    /// checked before any is paid. Empty means free.
    costs: Vec<EntityCastCost>,
    /// Requirements gating the transition, read the same way as a type's own
    /// [`requires`](crate::entity_type_def::EntityTypeDef::requires) list.
    /// Empty means always available.
    requires: Vec<String>,
}

impl MorphTransition {
    /// Creates a new `MorphTransition` with the given data.
    ///
    /// Panics if `into` or `via` is empty or `requires` contains an empty name.
    pub fn new(
        into: impl Into<String>,
        via: Option<&str>,
        time: MorphTime,
        placement: MorphPlacement,
        cancel: MorphCancel,
        costs: Vec<EntityCastCost>,
        requires: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let into = into.into();
        assert!(!into.is_empty(), "into must not be empty");
        let via = via.map(str::to_string);
        assert!(
            via.as_ref().is_none_or(|via| !via.is_empty()),
            "via must not be empty"
        );
        let requires: Vec<String> = requires.into_iter().map(Into::into).collect();
        assert!(
            requires.iter().all(|name| !name.is_empty()),
            "required names must not be empty"
        );

        Self {
            into,
            via,
            time,
            placement,
            cancel,
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
    pub fn time(&self) -> MorphTime {
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

    /// What starting the transition costs.
    #[inline]
    pub fn costs(&self) -> &[EntityCastCost] {
        &self.costs
    }

    /// Requirements gating the transition.
    #[inline]
    pub fn requires(&self) -> &[String] {
        &self.requires
    }
}
