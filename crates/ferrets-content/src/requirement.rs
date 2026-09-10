//! What must hold before an act — producing, researching, changing form,
//! casting — is allowed.

use crate::research::ResearchId;

/// Whom a requirement is asked of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The player acting.
    Player,
    /// The entity the act is asked of.
    Actor,
}

/// One entry of a requirement list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requirement {
    /// The player has a standing entity of this type, by registered name.
    EntityType(String),
    /// The player has a standing entity carrying this tag.
    Tag(String),
    /// The player has completed this research.
    Research(ResearchId),
    /// The acting entity has a standing annex of this type docked, by
    /// registered name.
    Annexed(String),
}

impl Requirement {
    /// Whom the entry is asked of.
    #[inline]
    pub fn scope(&self) -> Scope {
        match self {
            Requirement::EntityType(_) | Requirement::Tag(_) | Requirement::Research(_) => {
                Scope::Player
            }
            Requirement::Annexed(_) => Scope::Actor,
        }
    }
}
