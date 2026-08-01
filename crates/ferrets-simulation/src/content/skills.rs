//! The skill vocabulary: handles, the cast tree, and the activated abilities
//! content declares.
//!
//! A skill's caster kind is the arm of its [`SkillCaster`]: each arm carries
//! exactly the costs, targets, and effects that kind of caster can serve, so a
//! cast a caster cannot perform is unrepresentable rather than validated.

use ferrets_math::FixedU64;
use serde::{Deserialize, Serialize};

use super::entity_buffs::EntityBuffId;
use super::player_buffs::PlayerBuffId;
use crate::resources::Cost;

/// A handle to a registered skill, assigned in registration order.
///
/// Content declares skills by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SkillId(u16);

impl SkillId {
    /// Creates a skill id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more skills registered than SkillId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// How a skill is cast: by one of the issuing player's entities, or by the
/// player itself. Each arm carries the costs, targets, and effects its caster
/// kind can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCaster {
    /// An entity casts: its type must declare the skill, its cooldown is
    /// per entity, and pool costs draw from the caster's own pools.
    Entity {
        /// What one cast costs. Every entry must be payable and all are paid,
        /// so a cast never half-charges. Empty means free.
        costs: Vec<EntityCastCost>,
        /// Who the cast acts on.
        target: EntityCastTarget,
        /// What the cast does to the resolved target entity.
        effect: EntityCastEffect,
    },
    /// The issuing player casts: the cooldown is per player, only resources
    /// are payable, and the effect lands on the casting player.
    Player {
        /// Resource kinds one cast draws from the player's stockpile. Empty
        /// means free.
        cost: Cost,
        /// What the cast does to the casting player.
        effect: PlayerCastEffect,
    },
}

/// One price an entity cast pays, drawn from the pool its arm names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityCastCost {
    /// Resource kinds from the casting player's stockpile.
    Resources(Cost),
    /// The casting entity's energy pool.
    Energy(FixedU64),
    /// The casting entity's own health — a cast that could not be survived is
    /// refused.
    Health(FixedU64),
}

/// Who an entity cast acts on — always an entity. Allegiance is judged from
/// the caster's owner, and ally includes self.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityCastTarget {
    /// The casting entity itself; the use command carries no target.
    Caster,
    /// An owned or allied entity.
    Ally,
    /// A hostile entity.
    Enemy,
}

/// What a cast does to its resolved target entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityCastEffect {
    /// Applies the entity buff with the given id.
    ApplyBuff(EntityBuffId),
    /// Removes every active instance of the given entity buff.
    RemoveBuff(EntityBuffId),
    /// Deals flat damage, bypassing armor (like an ability, not a weapon).
    Damage(FixedU64),
    /// Restores health, up to the target's maximum.
    Heal(FixedU64),
}

/// What a cast does to the casting player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerCastEffect {
    /// Applies the player buff with the given id.
    ApplyBuff(PlayerBuffId),
    /// Removes every active instance of the given player buff.
    RemoveBuff(PlayerBuffId),
}

/// A content-defined skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDef {
    /// Ticks before the same caster can use the skill again.
    pub cooldown: u32,
    /// How the skill is cast, by whom, and what it does.
    pub caster: SkillCaster,
}
