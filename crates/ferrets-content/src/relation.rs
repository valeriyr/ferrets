//! How two owners stand toward each other, and the sets of those relations
//! that say whose something must be.

use std::ops::BitOr;

/// How two owners stand toward each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same player.
    Own,
    /// Two players sharing a team.
    Ally,
    /// Two players not allied.
    Enemy,
    /// At least one of the two is nobody's.
    Neutral,
}

impl Relation {
    /// Whether the two are one player.
    pub fn is_own(self) -> bool {
        match self {
            Relation::Own => true,
            Relation::Ally | Relation::Enemy | Relation::Neutral => false,
        }
    }

    /// Whether the two are one side: one player, or players sharing a team.
    pub fn is_friendly(self) -> bool {
        match self {
            Relation::Own | Relation::Ally => true,
            Relation::Enemy | Relation::Neutral => false,
        }
    }

    /// Whether the two are players not allied.
    pub fn is_enemy(self) -> bool {
        match self {
            Relation::Enemy => true,
            Relation::Own | Relation::Ally | Relation::Neutral => false,
        }
    }

    /// The bit this relation holds in a [`Relations`].
    const fn bit(self) -> u8 {
        match self {
            Relation::Own => 1,
            Relation::Ally => 1 << 1,
            Relation::Enemy => 1 << 2,
            Relation::Neutral => 1 << 3,
        }
    }
}

/// The relations a rule names: whose an entity, a coverage or a claim must
/// be, judged from the asker's owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relations(
    /// One bit per [`Relation`] the set names.
    u8,
);

impl Relations {
    /// The set that names no relation.
    pub const NONE: Relations = Relations(0);

    /// The set that names one side: the asker's own and its allies.
    pub const FRIENDLY: Relations = Relations(Relation::Own.bit() | Relation::Ally.bit());

    /// The set that names every relation, the unowned included.
    pub const EVERY: Relations = Relations(
        Relation::Own.bit()
            | Relation::Ally.bit()
            | Relation::Enemy.bit()
            | Relation::Neutral.bit(),
    );

    /// Whether the set names `relation`.
    pub fn contains(self, relation: Relation) -> bool {
        self.0 & relation.bit() != 0
    }

    /// Whether the set names no relation.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the set names any relation `other` names.
    pub fn intersects(self, other: Relations) -> bool {
        self.0 & other.0 != 0
    }

    /// The relations the set names, in [`Relation`]'s declared order.
    pub fn iter(self) -> impl Iterator<Item = Relation> {
        [
            Relation::Own,
            Relation::Ally,
            Relation::Enemy,
            Relation::Neutral,
        ]
        .into_iter()
        .filter(move |&relation| self.contains(relation))
    }
}

impl From<Relation> for Relations {
    /// The set naming `relation` alone.
    fn from(relation: Relation) -> Self {
        Relations(relation.bit())
    }
}

impl BitOr for Relations {
    type Output = Relations;

    fn bitor(self, other: Relations) -> Relations {
        Relations(self.0 | other.0)
    }
}

impl BitOr<Relation> for Relations {
    type Output = Relations;

    fn bitor(self, relation: Relation) -> Relations {
        self | Relations::from(relation)
    }
}

impl BitOr for Relation {
    type Output = Relations;

    fn bitor(self, other: Relation) -> Relations {
        Relations::from(self) | other
    }
}
