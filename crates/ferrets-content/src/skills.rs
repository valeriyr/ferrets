//! The skill vocabulary: handles, the cast tree, and the activated abilities
//! content declares.
//!
//! A skill's caster kind is the arm of its [`SkillCaster`]: each arm carries
//! exactly the costs, targets, and effects that kind of caster can serve, so a
//! cast a caster cannot perform is unrepresentable rather than validated.

use ferrets_math::FixedU64;
use serde::{Deserialize, Serialize};

use crate::{
    affiliation::Affiliation,
    costs::Cost,
    entity_buffs::EntityBuffId,
    entity_type_def::EntityTypeId,
    field::{FieldAction, FieldId},
    kinds::Kinds,
    player_buffs::PlayerBuffId,
    quantity::Quantity,
    requirement::Requirement,
};

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

/// How close a caster must be to what it casts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Wherever the caster stands: the cast lands the moment it is ordered,
    /// however far off the aim is.
    Wherever,
    /// Within the cells this comes to, which the caster walks into first.
    /// Zero is satisfied only by standing inside the aim's own footprint.
    Within(Quantity),
}

/// How long a cast holds its caster once it has settled into reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Casting {
    /// It lands the moment the caster is in reach, and takes no more of its
    /// time.
    Instant,
    /// It lands later: the effect arrives — and what it costs is paid — as the
    /// cast reaches `point`, and the caster is free again at `period`. A cast
    /// cut short before its point costs nothing.
    Delayed {
        /// Ticks before the cast lands.
        point: Quantity,
        /// Ticks the whole cast holds the caster, the landing included.
        period: Quantity,
    },
}

/// How a skill is cast: by one of the issuing player's entities, or by the
/// player itself. Each arm carries the costs, targets, reaches, and effects its
/// caster kind can serve.
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
        /// How close the caster must be to it.
        reach: Reach,
        /// How long the cast holds the caster once it is in reach.
        casting: Casting,
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

/// What an entity cast is aimed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityCastTarget {
    /// The casting entity itself; the use command carries no target.
    Caster,
    /// A cell of the map; the use command carries a position.
    Position,
    /// Something standing: whose it must be, and what it must be.
    Standing {
        /// Whose it must be.
        side: Affiliation,
        /// What it must be.
        kinds: Kinds,
    },
    /// Something lying where it fell — anyone's, since remains belong to
    /// nobody. The cast spends what it is aimed at.
    Fallen {
        /// What it must be.
        kinds: Kinds,
    },
}

/// What a cast does at its resolved aim.
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
    /// Puts a patch of map in the casting player's sight for a while,
    /// whatever stands there: the aimed cell for a position cast, the cell the
    /// target's position falls in otherwise.
    Watch {
        /// How far from the aim the sight reaches, in cells.
        radius: u32,
        /// Ticks it lasts.
        duration: u32,
    },
    /// Sets units down around the aim, for the casting player: on the cells
    /// nearest the remains a raise spends, the caster's own footprint, or the
    /// aimed cell.
    Summon {
        /// The type summoned.
        entity_type: EntityTypeId,
        /// How many. A cast with nowhere to put them all does nothing.
        count: u32,
    },
    /// Covers or clears a field around the aim: the aimed cell for a
    /// position cast, the cell the target's position falls in otherwise.
    Field {
        /// The field acted on.
        field: FieldId,
        /// How far from the aim the action reaches, in cells.
        radius: u32,
        /// Whether the cells are covered or cleared.
        action: FieldAction,
    },
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
    /// Requirements for casting, read the same way as a type's own
    /// [`requires`](crate::entity_type_def::EntityTypeDef::requires) list: a
    /// player-scoped entry is asked of the caster's owner, an actor-scoped one
    /// of the caster.
    pub requires: Vec<Requirement>,
}
