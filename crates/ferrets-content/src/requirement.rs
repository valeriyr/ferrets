//! What must hold of a player acting through an entity — before an act
//! (producing, researching, changing form, casting) is allowed, or for as long
//! as a buff is borne.

use ferrets_math::FixedU64;

use crate::{entity_stats::EntityStatId, research::ResearchId};

/// Whom a requirement is asked of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The player acting.
    Player,
    /// The entity the act is asked of.
    Actor,
}

/// Which side of a number a requirement asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    /// Strictly under it.
    Under(FixedU64),
    /// At it or over it.
    AtLeast(FixedU64),
}

impl Threshold {
    /// The number the threshold is drawn at.
    #[inline]
    pub fn number(self) -> FixedU64 {
        match self {
            Threshold::Under(number) | Threshold::AtLeast(number) => number,
        }
    }
}

/// A limit a requirement compares against: a plain amount, or a share of the
/// reference the requirement names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// The threshold's number is the amount itself.
    Amount(Threshold),
    /// The threshold's number is a fraction of the reference: `0.5` is half of
    /// it, `1` all of it.
    Share(Threshold),
}

impl Bound {
    /// Whether `value` lies on this side of the bound, a share being taken of
    /// `reference`.
    #[inline]
    pub fn admits(self, value: FixedU64, reference: FixedU64) -> bool {
        let limit = match self {
            Bound::Amount(threshold) => threshold.number(),
            Bound::Share(threshold) => threshold.number().saturating_mul(reference),
        };
        match self.threshold() {
            Threshold::Under(_) => value < limit,
            Threshold::AtLeast(_) => value >= limit,
        }
    }

    /// The side and the number, whichever way the number is read.
    #[inline]
    pub fn threshold(self) -> Threshold {
        match self {
            Bound::Amount(threshold) | Bound::Share(threshold) => threshold,
        }
    }
}

/// One requirement: a leaf asked of the player or the actor, or a node over
/// other requirements. A list of requirements reads as `All`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requirement {
    /// Every one is met.
    All(Vec<Requirement>),
    /// At least one is met.
    Any(Vec<Requirement>),
    /// The requirement it holds is not met.
    Unless(Box<Requirement>),
    /// The player has a standing entity of this type, by registered name.
    EntityType(String),
    /// The player has a standing entity carrying this tag.
    Tag(String),
    /// The player has completed this research.
    Research(ResearchId),
    /// The acting entity has a standing annex of this type docked, by
    /// registered name.
    Annexed(String),
    /// The acting entity's health pool lies on this side of the bound, a
    /// share being of its effective maximum.
    Health(Bound),
    /// The acting entity's energy pool lies on this side of the bound, a
    /// share being of its effective maximum.
    Energy(Bound),
    /// One of the acting entity's effective stats lies on this side of the
    /// bound, a share being of the stat's base.
    Stat {
        /// The stat read.
        stat: EntityStatId,
        /// The side asked for.
        bound: Bound,
    },
    /// The acting entity is not under construction.
    Built,
    /// The acting entity takes orders and runs none.
    Idle,
    /// The acting entity takes orders, runs none, and has run none for at
    /// least this many ticks.
    IdleFor(u32),
    /// No hit has landed on the acting entity for this many ticks, or ever.
    UnhurtFor(u32),
}

impl Requirement {
    /// Whom the requirement is asked of: a node takes the widest scope of its
    /// children, the player's when none asks of the actor.
    pub fn scope(&self) -> Scope {
        match self {
            Requirement::All(items) | Requirement::Any(items) => {
                items
                    .iter()
                    .fold(Scope::Player, |widest, item| match (widest, item.scope()) {
                        (Scope::Actor, _) | (_, Scope::Actor) => Scope::Actor,
                        (Scope::Player, Scope::Player) => Scope::Player,
                    })
            }
            Requirement::Unless(item) => item.scope(),
            Requirement::EntityType(_) | Requirement::Tag(_) | Requirement::Research(_) => {
                Scope::Player
            }
            Requirement::Annexed(_)
            | Requirement::Health(_)
            | Requirement::Energy(_)
            | Requirement::Stat { .. }
            | Requirement::Built
            | Requirement::Idle
            | Requirement::IdleFor(_)
            | Requirement::UnhurtFor(_) => Scope::Actor,
        }
    }

    /// Every leaf under this requirement, in content order, nodes flattened.
    pub fn leaves(&self) -> Vec<&Requirement> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a Requirement>) {
        match self {
            Requirement::All(items) | Requirement::Any(items) => {
                for item in items {
                    item.collect_leaves(out);
                }
            }
            Requirement::Unless(item) => item.collect_leaves(out),
            Requirement::EntityType(_)
            | Requirement::Tag(_)
            | Requirement::Research(_)
            | Requirement::Annexed(_)
            | Requirement::Health(_)
            | Requirement::Energy(_)
            | Requirement::Stat { .. }
            | Requirement::Built
            | Requirement::Idle
            | Requirement::IdleFor(_)
            | Requirement::UnhurtFor(_) => out.push(self),
        }
    }
}
