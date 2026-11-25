//! How two owners stand toward each other.

use ferrets_content::affiliation::Affiliation;

/// How two owners stand toward each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same player.
    Own,
    /// Two players sharing a team.
    Allied,
    /// Two players not allied.
    Enemy,
    /// At least one of the two is nobody's.
    Neutral,
}

impl Relation {
    /// Whether a rule reading `affiliation` names this relation: the "whose"
    /// every capability declares. A neutral pair fits [`Affiliation::Anyone`]
    /// alone.
    pub fn fits(self, affiliation: Affiliation) -> bool {
        match affiliation {
            Affiliation::Own => self.is_own(),
            Affiliation::Allied => self.is_friendly(),
            Affiliation::Enemy => self.is_enemy(),
            Affiliation::Anyone => true,
        }
    }

    /// Whether the two are one player.
    pub fn is_own(self) -> bool {
        match self {
            Relation::Own => true,
            Relation::Allied | Relation::Enemy | Relation::Neutral => false,
        }
    }

    /// Whether the two are one side: one player, or players sharing a team.
    pub fn is_friendly(self) -> bool {
        match self {
            Relation::Own | Relation::Allied => true,
            Relation::Enemy | Relation::Neutral => false,
        }
    }

    /// Whether the two are players not allied.
    pub fn is_enemy(self) -> bool {
        match self {
            Relation::Enemy => true,
            Relation::Own | Relation::Allied | Relation::Neutral => false,
        }
    }
}
