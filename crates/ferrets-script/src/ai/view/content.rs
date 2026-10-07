//! The static content catalogue a script can consult, snapshotted once per
//! session.

use ferrets_content::{
    affiliation::Affiliation,
    cost::Cost,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    quantity::Quantity,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    research::ResearchId,
    skills::{EntityCastTarget, SkillCaster, SkillId},
};

/// The limit a requirement compares against, as a script reads it: an amount,
/// or a fraction of the reference, as a decimal string.
pub enum BoundView {
    /// Strictly under the amount.
    Under(String),
    /// At the amount or over it.
    AtLeast(String),
    /// Strictly under the share.
    UnderShare(String),
    /// At the share or over it.
    AtLeastShare(String),
}

/// One requirement as a script reads it.
pub enum RequirementView {
    /// Every one is met.
    All(Vec<RequirementView>),
    /// At least one is met.
    Any(Vec<RequirementView>),
    /// The requirement it holds is not met.
    Unless(Box<RequirementView>),
    /// A registered thing the player or the actor must have: the kind is
    /// `"entity_type"`, `"tag"`, `"research"`, or `"annexed"`.
    Named {
        /// The kind of thing wanted.
        kind: String,
        /// Its registered name.
        name: String,
    },
    /// A pool of the actor's, `"health"` or `"energy"`, against a bound: an
    /// amount of it, or a share of its maximum.
    Pool {
        /// Which pool.
        kind: String,
        /// The side asked for.
        bound: BoundView,
    },
    /// One of the actor's effective stats, by registered name, against a
    /// bound: an amount, or a share of the stat's base.
    Stat {
        /// The stat read.
        name: String,
        /// The side asked for.
        bound: BoundView,
    },
    /// The actor is not under construction.
    Built,
    /// The actor takes orders and runs none.
    Idle,
    /// The actor takes orders, runs none, and has run none for at least this
    /// many ticks.
    IdleFor(u32),
    /// No hit has landed on the actor for this many ticks, or ever.
    UnhurtFor(u32),
}

/// The static content catalogue a script can consult.
pub struct ContentView {
    /// Registered resource kinds, in ascending order.
    pub resources: Vec<String>,
    pub entities: Vec<EntityContentView>,
    pub researches: Vec<ResearchContentView>,
    pub skills: Vec<SkillContentView>,
}

impl ContentView {
    /// Snapshots the registered content, in ascending name order.
    pub fn from_registry(registry: &ContentRegistry) -> ContentView {
        ContentView {
            resources: registry.resources().map(str::to_string).collect(),
            entities: registry
                .entities()
                .map(|def| EntityContentView::from_def(def, registry))
                .collect(),
            researches: registry
                .researches()
                .map(|(name, id)| {
                    let def = registry
                        .research_def(id)
                        .expect("a listed research resolves in its own registry");
                    ResearchContentView {
                        name: name.to_string(),
                        id,
                        price: def
                            .price
                            .iter()
                            .map(|(kind, amount)| (kind.clone(), *amount))
                            .collect(),
                        time: def.research_time,
                        requires: requirements(&def.requires, registry),
                    }
                })
                .collect(),
            skills: registry
                .skills()
                .map(|(name, id)| {
                    let def = registry
                        .skill_def(id)
                        .expect("a listed skill resolves in its own registry");
                    let (caster, target) = match &def.caster {
                        SkillCaster::Entity { target, .. } => (
                            "entity",
                            Some(match target {
                                EntityCastTarget::Caster => "caster",
                                EntityCastTarget::Position => "position",
                                EntityCastTarget::Fallen { .. } => "fallen",
                                EntityCastTarget::Standing { side, .. } => match side {
                                    Affiliation::Own => "own",
                                    Affiliation::Allied => "allied",
                                    Affiliation::Enemy => "enemy",
                                    Affiliation::Anyone => "anyone",
                                },
                            }),
                        ),
                        SkillCaster::Player { .. } => ("player", None),
                    };
                    SkillContentView {
                        name: name.to_string(),
                        id,
                        caster: caster.to_string(),
                        target: target.map(str::to_string),
                        requires: requirements(&def.requires, registry),
                    }
                })
                .collect(),
        }
    }
}

/// The fields of one skill definition a script can consult.
pub struct SkillContentView {
    pub name: String,
    /// The registry handle the name resolves to, for the command boundary —
    /// scripts name skills, commands carry ids.
    pub id: SkillId,
    /// Which arm casts: `"entity"` or `"player"`.
    pub caster: String,
    /// Who an entity cast acts on: `"caster"`, `"ally"`, or `"enemy"`. `None`
    /// for a player cast, which lands on the casting player.
    pub target: Option<String>,
    /// What must hold for a cast. `None` when nothing must.
    pub requires: Option<Vec<RequirementView>>,
}

/// The fields of one research definition a script can consult.
pub struct ResearchContentView {
    pub name: String,
    /// The registry handle the name resolves to, for the command boundary —
    /// scripts name researches, commands carry ids.
    pub id: ResearchId,
    /// Price per resource kind, in ascending kind order. Empty means free.
    pub price: Vec<(String, u32)>,
    /// Ticks a researcher works to complete the research.
    pub time: u32,
    /// What must hold to start it. `None` when nothing must.
    pub requires: Option<Vec<RequirementView>>,
}

/// The fields of one entity type definition a script can consult.
pub struct EntityContentView {
    pub name: String,
    /// Price per resource kind, in ascending kind order. Empty means free.
    pub price: Vec<(String, u32)>,
    pub train_time: Option<u32>,
    pub build_time: Option<u32>,
    /// Trainable types. `None` when instances cannot train.
    pub trains: Option<Vec<String>>,
    /// Hostable researches by name. `None` when instances cannot research.
    pub researches: Option<Vec<String>>,
    /// Castable skills by name. `None` when instances have none.
    pub skills: Option<Vec<String>>,
    /// What must hold to produce one. `None` when nothing must.
    pub requires: Option<Vec<RequirementView>>,
    /// Constructible types. `None` when instances cannot build.
    pub builds: Option<Vec<String>>,
    /// Footprint width and height in cells.
    pub size: (u32, u32),
    /// Maximum health (the `max_health` stat). `None` when the type has none.
    pub max_health: Option<u32>,
    /// The weapon. `None` when the type cannot attack.
    pub attack: Option<AttackView>,
    /// Harvestable resource kinds. `None` when the type cannot harvest.
    pub harvests: Option<Vec<String>>,
    /// Resource kinds accepted for delivery. `None` when not a storage.
    pub stores: Option<Vec<String>>,
    pub can_move: bool,
    /// Changes of form instances can start, in declaration order. `None`
    /// when the type declares none.
    pub morphs: Option<Vec<MorphView>>,
    /// What instances breed. `None` when the type breeds nothing.
    pub breeder: Option<BreederView>,
}

/// What a type breeds, and how many at once.
pub struct BreederView {
    /// The type bred.
    pub breeds: String,
    /// How many broodlings live at once.
    pub limit: u32,
    /// Ticks between births. `None` when the period is read from a stat.
    pub period: Option<u32>,
}

/// One change of form a type declares.
pub struct MorphView {
    /// The type the change lands as.
    pub into: String,
    /// The stockpile price per resource kind, in ascending kind order. Empty
    /// means the change draws nothing from the stockpile.
    pub price: Vec<(String, u32)>,
    /// Ticks the change takes. `None` when the time is read from a stat.
    pub time: Option<u32>,
}

/// A type's weapon — the combat stats a script reads together (a type carries
/// them all, or has no weapon at all).
pub struct AttackView {
    pub damage: u32,
    pub attack_range: u32,
}

impl EntityContentView {
    /// Snapshots the fields a script can consult from one type definition.
    pub fn from_def(def: &EntityTypeDef, registry: &ContentRegistry) -> EntityContentView {
        EntityContentView {
            name: def.name.clone(),
            price: def
                .price
                .iter()
                .map(|(kind, amount)| (kind.clone(), *amount))
                .collect(),
            train_time: def.train_time,
            build_time: def.build_time(),
            trains: def
                .trainer
                .as_ref()
                .map(|t| t.trains().map(str::to_string).collect()),
            researches: def.researcher.as_ref().map(|r| {
                r.researches()
                    .filter_map(|id| registry.research_name(id).map(str::to_string))
                    .collect()
            }),
            skills: (!def.skills.is_empty()).then(|| {
                def.skills
                    .iter()
                    .filter_map(|&id| registry.skill_name(id).map(str::to_string))
                    .collect()
            }),
            requires: requirements(&def.requires, registry),
            builds: def
                .builder
                .as_ref()
                .map(|b| b.builds().map(str::to_string).collect()),
            size: def
                .location
                .as_ref()
                .map_or((1, 1), |l| (l.size().width, l.size().height)),
            max_health: def.base_stat_as_u32(EntityStatId::MAX_HEALTH),
            attack: def
                .base_stat(EntityStatId::DAMAGE)
                .zip(def.base_stat(EntityStatId::ATTACK_RANGE))
                .map(|(damage, range)| AttackView {
                    damage: damage.to_num::<u32>(),
                    attack_range: range.to_num::<u32>(),
                }),
            harvests: def
                .resource_carrier
                .as_ref()
                .map(|c| c.kinds().map(str::to_string).collect()),
            stores: def
                .resource_storage
                .as_ref()
                .map(|s| s.kinds().map(str::to_string).collect()),
            can_move: def.can_move(),
            morphs: (!def.morphs.is_empty()).then(|| {
                def.morphs
                    .iter()
                    .map(|transition| MorphView {
                        into: transition.into_type().to_string(),
                        price: transition
                            .costs()
                            .iter()
                            .flat_map(|cost| match cost {
                                Cost::Resources(resources) => resources
                                    .iter()
                                    .map(|(kind, amount)| (kind.clone(), *amount))
                                    .collect(),
                                Cost::Energy(_) | Cost::Health(_) => Vec::new(),
                            })
                            .collect(),
                        time: match transition.time() {
                            Quantity::Constant(ticks) => Some(ticks),
                            Quantity::Stat(_) => None,
                        },
                    })
                    .collect()
            }),
            breeder: def.breeder.as_ref().map(|brood| BreederView {
                breeds: brood.breeds().to_string(),
                limit: u32::try_from(brood.limit()).expect("a brood limit fits in u32"),
                period: match brood.period() {
                    Quantity::Constant(ticks) => Some(ticks),
                    Quantity::Stat(_) => None,
                },
            }),
        }
    }
}

/// The entries of `requires` as a script reads them, or `None` when it holds
/// none.
fn requirements(
    requires: &[Requirement],
    registry: &ContentRegistry,
) -> Option<Vec<RequirementView>> {
    let views: Vec<RequirementView> = requires
        .iter()
        .map(|entry| requirement(entry, registry))
        .collect();
    (!views.is_empty()).then_some(views)
}

/// One requirement as a script reads it.
fn requirement(entry: &Requirement, registry: &ContentRegistry) -> RequirementView {
    let named = |kind: &str, name: String| RequirementView::Named {
        kind: kind.to_string(),
        name,
    };
    match entry {
        Requirement::All(items) => RequirementView::All(
            items
                .iter()
                .map(|item| requirement(item, registry))
                .collect(),
        ),
        Requirement::Any(items) => RequirementView::Any(
            items
                .iter()
                .map(|item| requirement(item, registry))
                .collect(),
        ),
        Requirement::Unless(item) => RequirementView::Unless(Box::new(requirement(item, registry))),
        Requirement::EntityType(name) => named("entity_type", name.clone()),
        Requirement::Tag(name) => named("tag", name.clone()),
        Requirement::Research(research) => named(
            "research",
            registry
                .research_name(*research)
                .expect("a requirement's research id was minted by this registry")
                .to_string(),
        ),
        Requirement::Annexed(name) => named("annexed", name.clone()),
        Requirement::Health(bound) => RequirementView::Pool {
            kind: "health".to_string(),
            bound: bound_view(*bound),
        },
        Requirement::Energy(bound) => RequirementView::Pool {
            kind: "energy".to_string(),
            bound: bound_view(*bound),
        },
        Requirement::Stat { stat, bound } => RequirementView::Stat {
            name: registry
                .entity_stat_name(*stat)
                .expect("a requirement's stat id was registered by this registry")
                .to_string(),
            bound: bound_view(*bound),
        },
        Requirement::Built => RequirementView::Built,
        Requirement::Idle => RequirementView::Idle,
        Requirement::IdleFor(ticks) => RequirementView::IdleFor(*ticks),
        Requirement::UnhurtFor(ticks) => RequirementView::UnhurtFor(*ticks),
    }
}

/// A bound as a script reads it.
fn bound_view(bound: Bound) -> BoundView {
    match bound {
        Bound::Amount(Threshold::Under(value)) => BoundView::Under(value.to_string()),
        Bound::Amount(Threshold::AtLeast(value)) => BoundView::AtLeast(value.to_string()),
        Bound::Share(Threshold::Under(value)) => BoundView::UnderShare(value.to_string()),
        Bound::Share(Threshold::AtLeast(value)) => BoundView::AtLeastShare(value.to_string()),
    }
}
