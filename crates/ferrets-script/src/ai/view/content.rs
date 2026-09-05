//! The static content catalogue a script can consult, snapshotted once per
//! session.

use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    morph::MorphTime,
    registry::ContentRegistry,
    research::ResearchId,
    skills::{EntityCastCost, EntityCastTarget, SkillCaster, SkillId},
};

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
                        cost: def
                            .cost
                            .iter()
                            .map(|(kind, amount)| (kind.clone(), *amount))
                            .collect(),
                        time: def.research_time,
                        requires: (!def.requires.is_empty()).then(|| def.requires.clone()),
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
                                EntityCastTarget::Ally => "ally",
                                EntityCastTarget::Enemy => "enemy",
                                EntityCastTarget::Position => "position",
                            }),
                        ),
                        SkillCaster::Player { .. } => ("player", None),
                    };
                    SkillContentView {
                        name: name.to_string(),
                        id,
                        caster: caster.to_string(),
                        target: target.map(str::to_string),
                        requires: (!def.requires.is_empty()).then(|| def.requires.clone()),
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
    /// Requirements for casting. `None` when always available.
    pub requires: Option<Vec<String>>,
}

/// The fields of one research definition a script can consult.
pub struct ResearchContentView {
    pub name: String,
    /// The registry handle the name resolves to, for the command boundary —
    /// scripts name researches, commands carry ids.
    pub id: ResearchId,
    /// Price per resource kind, in ascending kind order. Empty means free.
    pub cost: Vec<(String, u32)>,
    /// Ticks a researcher works to complete the research.
    pub time: u32,
    /// Requirements for starting the research. `None` when always available.
    pub requires: Option<Vec<String>>,
}

/// The fields of one entity type definition a script can consult.
pub struct EntityContentView {
    pub name: String,
    /// Price per resource kind, in ascending kind order. Empty means free.
    pub cost: Vec<(String, u32)>,
    pub train_time: Option<u32>,
    pub build_time: Option<u32>,
    /// Trainable types. `None` when instances cannot train.
    pub trains: Option<Vec<String>>,
    /// Hostable researches by name. `None` when instances cannot research.
    pub researches: Option<Vec<String>>,
    /// Castable skills by name. `None` when instances have none.
    pub skills: Option<Vec<String>>,
    /// Requirements for producing an instance. `None` when always available.
    pub requires: Option<Vec<String>>,
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
}

/// One change of form a type declares.
pub struct MorphView {
    /// The type the change lands as.
    pub into: String,
    /// The stockpile price per resource kind, in ascending kind order. Empty
    /// means the change draws nothing from the stockpile.
    pub cost: Vec<(String, u32)>,
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
            cost: def
                .cost
                .iter()
                .map(|(kind, amount)| (kind.clone(), *amount))
                .collect(),
            train_time: def.train_time,
            build_time: def.build_time,
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
            requires: (!def.requires.is_empty()).then(|| def.requires.clone()),
            builds: def
                .builder
                .as_ref()
                .map(|b| b.builds().map(str::to_string).collect()),
            size: def
                .location
                .as_ref()
                .map_or((1, 1), |l| (l.size().width, l.size().height)),
            max_health: def
                .base_stat(EntityStatId::MAX_HEALTH)
                .map(|v| v.to_num::<u32>()),
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
                        cost: transition
                            .costs()
                            .iter()
                            .flat_map(|cost| match cost {
                                EntityCastCost::Resources(resources) => resources
                                    .iter()
                                    .map(|(kind, amount)| (kind.clone(), *amount))
                                    .collect(),
                                EntityCastCost::Energy(_) | EntityCastCost::Health(_) => Vec::new(),
                            })
                            .collect(),
                        time: match transition.time() {
                            MorphTime::Constant(ticks) => Some(ticks),
                            MorphTime::Stat(_) => None,
                        },
                    })
                    .collect()
            }),
        }
    }
}
