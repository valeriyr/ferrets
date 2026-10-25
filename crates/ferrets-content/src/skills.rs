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
    cost::Cost,
    detection::Detection,
    entity_buffs::EntityBuffId,
    entity_type_def::EntityTypeId,
    field::{FieldAction, FieldId, FieldPlacement},
    kinds::Kinds,
    landing::Landing,
    player_buffs::PlayerBuffId,
    price::Price,
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
    /// It lands later: the effect arrives as the cast reaches `point`, and
    /// the caster is free again at `period`. Once casting has started, a tick
    /// its aim cannot be held — out of reach, gone or no longer seen — breaks
    /// the cast.
    Delayed {
        /// Ticks before the cast lands.
        point: Quantity,
        /// Ticks the whole cast holds the caster, the landing included.
        period: Quantity,
        /// When its costs are paid and its cooldown starts.
        pays: PaymentPoint,
    },
}

/// When a delayed cast pays its costs and starts its cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentPoint {
    /// When the casting starts, on its first tick of work after the walk; what
    /// a cast cut short before it lands gives back.
    AtStart(Refund),
    /// When it lands: a cast cut short before then has cost nothing.
    AtLanding,
}

/// What a cast paid at its start gives back when it is cut short before it
/// lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refund {
    /// Nothing: what was paid stays spent, and the cooldown runs on.
    None,
    /// Everything: every cost comes back, and the cooldown is reset.
    Full,
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
        costs: Vec<Cost>,
        /// How long the cast holds the caster once it is in reach.
        casting: Casting,
        /// What the cast does, and what it is aimed at.
        effect: EntityCastEffect,
    },
    /// The issuing player casts: the cooldown is per player, only resources
    /// are payable, and the effect lands on the casting player.
    Player {
        /// Resource kinds one cast draws from the player's stockpile. Empty
        /// means free.
        price: Price,
        /// What the cast does to the casting player.
        effect: PlayerCastEffect,
    },
}

/// What an entity cast is aimed at, read off its effect: what the use
/// command names, and how close the caster must come to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityCastTarget<'a> {
    /// The casting entity itself; the use command carries no target.
    Caster,
    /// A cell of the map; the use command carries a position.
    Position {
        /// How close the caster must come to the cell.
        reach: Reach,
    },
    /// Something standing on the map — a unit or a building, not remains;
    /// the use command carries its id.
    Standing {
        /// Whose and what it must be.
        whom: &'a Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// Something lying where it fell — anyone's, since remains belong to
    /// nobody. The cast spends what it is aimed at.
    Fallen {
        /// What it must be.
        kinds: &'a Kinds,
        /// How close the caster must come to it.
        reach: Reach,
    },
}

/// Whose and what an aimed entity must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whom {
    /// Whose it must be, judged from the caster's owner.
    pub side: Affiliation,
    /// What it must be.
    pub kinds: Kinds,
}

/// The entity a cast acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// The casting entity itself.
    Caster,
    /// An aimed entity standing on the map.
    Standing {
        /// Whose and what it must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
}

impl Recipient {
    /// What a cast on this recipient is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            Recipient::Caster => EntityCastTarget::Caster,
            Recipient::Standing { whom, reach } => EntityCastTarget::Standing {
                whom,
                reach: *reach,
            },
        }
    }
}

/// The spot a cast acts around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spot {
    /// The cell the caster's position falls in.
    Caster,
    /// An aimed cell.
    Cell {
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// The cell an aimed standing entity's position falls in.
    Standing {
        /// Whose and what it must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
}

impl Spot {
    /// What a cast around this spot is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            Spot::Caster => EntityCastTarget::Caster,
            Spot::Cell { reach } => EntityCastTarget::Position { reach: *reach },
            Spot::Standing { whom, reach } => EntityCastTarget::Standing {
                whom,
                reach: *reach,
            },
        }
    }
}

/// How a summoned body arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonArrival {
    /// Standing, at once.
    Standing,
    /// Rising unattended over its type's `rise`, the caster its founder.
    Rising,
}

/// Which cell a summoned body is set down on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonPlacement {
    /// Anchored on the aimed cell — for a type raised over a resource source,
    /// on the source under it — or the cast is refused.
    Exact,
    /// The nearest cell around the aim that takes it.
    Nearby,
}

/// Where a summon sets its units down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummonAt {
    /// Around the caster's own footprint.
    Caster,
    /// Around an aimed cell.
    Cell {
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// Around an aimed standing entity's footprint.
    Standing {
        /// Whose and what it must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// Around aimed remains, which the cast spends.
    Fallen {
        /// What they must be.
        kinds: Kinds,
        /// How close the caster must come to them.
        reach: Reach,
    },
}

impl SummonAt {
    /// What a summon here is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            SummonAt::Caster => EntityCastTarget::Caster,
            SummonAt::Cell { reach } => EntityCastTarget::Position { reach: *reach },
            SummonAt::Standing { whom, reach } => EntityCastTarget::Standing {
                whom,
                reach: *reach,
            },
            SummonAt::Fallen { kinds, reach } => EntityCastTarget::Fallen {
                kinds,
                reach: *reach,
            },
        }
    }
}

/// An aim away from the caster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Away {
    /// An aimed cell.
    Cell {
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// An aimed standing entity's footprint.
    Standing {
        /// Whose and what it must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
}

impl Away {
    /// What a cast at this aim is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            Away::Cell { reach } => EntityCastTarget::Position { reach: *reach },
            Away::Standing { whom, reach } => EntityCastTarget::Standing {
                whom,
                reach: *reach,
            },
        }
    }
}

/// What a cast does at its resolved aim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityCastEffect {
    /// Applies an entity buff.
    ApplyBuff {
        /// The buff applied.
        buff: EntityBuffId,
        /// Who bears it.
        on: Recipient,
    },
    /// Removes every active instance of an entity buff.
    RemoveBuff {
        /// The buff removed.
        buff: EntityBuffId,
        /// Who loses it.
        on: Recipient,
    },
    /// Deals flat damage, bypassing armor (like an ability, not a weapon).
    Damage {
        /// How much.
        amount: FixedU64,
        /// Who takes it.
        on: Recipient,
    },
    /// Restores health, up to the recipient's maximum.
    Heal {
        /// How much.
        amount: FixedU64,
        /// Who is healed.
        on: Recipient,
    },
    /// Puts a patch of map in the casting player's sight for a while,
    /// whatever stands there.
    Watch {
        /// The spot the patch lies around.
        at: Spot,
        /// How far from the spot the sight reaches, in cells.
        radius: Quantity,
        /// Ticks it lasts.
        duration: Quantity,
        /// What it does for the casting player against the concealed
        /// entities standing on the patch.
        detection: Detection,
    },
    /// Sets units down for the casting player where they are summoned.
    Summon {
        /// Where they are set down.
        at: SummonAt,
        /// The type summoned.
        entity_type: EntityTypeId,
        /// How many. A cast with nowhere to put them all does nothing.
        count: Quantity,
        /// Which cell around the aim each takes.
        placement: SummonPlacement,
        /// Which cells the casting side must know of to set them down on.
        landing: Landing,
        /// The field rules their cells must meet for their owner, where they
        /// are set down and, for those that rise, until they have risen.
        /// Empty means fields never gate where they land.
        within: Vec<FieldPlacement>,
        /// Whether they stand at once or rise.
        arrives: SummonArrival,
    },
    /// Covers or clears a field around a spot.
    Field {
        /// The spot the action reaches around.
        at: Spot,
        /// The field acted on.
        field: FieldId,
        /// How far from the spot the action reaches, in cells.
        radius: Quantity,
        /// Whether the cells are covered or cleared.
        action: FieldAction,
    },
    /// Moves bodies standing on the map to new cells at once, each on the
    /// free cell nearest the destination it fits.
    Relocate {
        /// Who is moved, and where to.
        journey: Box<Journey>,
        /// Which cells they may be set down on.
        landing: Landing,
    },
}

impl EntityCastEffect {
    /// What the cast is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            EntityCastEffect::ApplyBuff { on, .. }
            | EntityCastEffect::RemoveBuff { on, .. }
            | EntityCastEffect::Damage { on, .. }
            | EntityCastEffect::Heal { on, .. } => on.target(),
            EntityCastEffect::Watch { at, .. } | EntityCastEffect::Field { at, .. } => at.target(),
            EntityCastEffect::Summon { at, .. } => at.target(),
            EntityCastEffect::Relocate { journey, .. } => journey.target(),
        }
    }
}

/// Who a relocation moves, and where to. A journey's destination is never
/// among the bodies it gathers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Journey {
    /// A party gathered at the caster, and where it goes.
    FromCaster {
        /// Who is gathered at the caster.
        party: CasterParty,
        /// Where the party goes.
        to: FromCasterTo,
    },
    /// A party gathered at the aim, and where it goes.
    FromAim {
        /// Who is gathered at the aim.
        party: AimParty,
        /// Where the party goes.
        to: FromAimTo,
    },
    /// The caster and the aimed entity trade places.
    Swap {
        /// Whose and what the aimed entity must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
}

impl Journey {
    /// The bodies the journey gathers around a spot, if its party is a
    /// gathering.
    pub fn gathering(&self) -> Option<&Gathering> {
        match self {
            Journey::FromCaster {
                party: CasterParty::CasterAndAround(gathering) | CasterParty::Around(gathering),
                ..
            }
            | Journey::FromAim {
                party: AimParty::Around { gathering, .. },
                ..
            } => Some(gathering),
            Journey::FromCaster {
                party: CasterParty::Caster,
                ..
            }
            | Journey::FromAim {
                party: AimParty::Target { .. },
                ..
            }
            | Journey::Swap { .. } => None,
        }
    }

    /// What a cast on this journey is aimed at.
    pub fn target(&self) -> EntityCastTarget<'_> {
        match self {
            Journey::FromCaster {
                to: FromCasterTo::Aim(away),
                ..
            }
            | Journey::FromAim {
                party: AimParty::Around { at: away, .. },
                ..
            } => away.target(),
            Journey::FromCaster {
                to: FromCasterTo::Home(_),
                ..
            } => EntityCastTarget::Caster,
            Journey::FromAim {
                party: AimParty::Target { whom, reach },
                ..
            }
            | Journey::Swap { whom, reach } => EntityCastTarget::Standing {
                whom,
                reach: *reach,
            },
        }
    }
}

/// Who a relocation gathers at the caster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasterParty {
    /// The caster itself.
    Caster,
    /// The caster and the bodies gathered around it.
    CasterAndAround(Gathering),
    /// The bodies gathered around the caster, without it.
    Around(Gathering),
}

/// Who a relocation gathers at the aim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AimParty {
    /// The aimed entity.
    Target {
        /// Whose and what it must be.
        whom: Whom,
        /// How close the caster must come to it.
        reach: Reach,
    },
    /// The bodies gathered around the aim.
    Around {
        /// The aim they stand around.
        at: Away,
        /// Which bodies are gathered.
        gathering: Gathering,
    },
}

/// Where a party gathered at the caster goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromCasterTo {
    /// The aim.
    Aim(Away),
    /// The caster's owner's building of these kinds nearest the caster.
    Home(Kinds),
}

/// Where a party gathered at the aim goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromAimTo {
    /// The caster.
    Caster,
    /// The caster's owner's building of these kinds nearest the aim.
    Home(Kinds),
}

/// The bodies a relocation gathers around a spot, nearest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gathering {
    /// How far from the spot they may stand, in cells.
    pub radius: Quantity,
    /// Whose they must be, judged from the caster's owner.
    pub side: Affiliation,
    /// What they must be.
    pub kinds: Kinds,
    /// Whether the caster's side must make each out.
    pub sight: GatherSight,
    /// How many at most.
    pub count: Count,
    /// What the cast does when some of them fit nowhere.
    pub shortfall: Shortfall,
}

/// Whether a gathering reads what the caster's side makes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatherSight {
    /// Every body that qualifies, made out or not.
    Blind,
    /// Only bodies the caster's side makes out: in its sight, and detected
    /// where concealed.
    Seen,
}

/// How many bodies a gathering takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    /// Every one that qualifies.
    All,
    /// The nearest ones, up to this many.
    AtMost(Quantity),
}

/// What a relocation does when some of the bodies it gathers fit nowhere
/// near the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortfall {
    /// The whole cast is refused before anything is paid.
    Refuse,
    /// Those that fit are moved; the rest stay where they stand.
    Leave,
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
