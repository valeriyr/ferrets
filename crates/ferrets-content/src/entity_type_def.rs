//! Definition of a single entity type — the content-level blueprint for spawning.

use ferrets_geometry::cell_size::CellSize;
use std::collections::{BTreeMap, BTreeSet};

use ferrets_math::FixedU64;
use ferrets_pathfinder::layer_mask::LayerMask;

use crate::{
    attack::{AttackDef, Delivery, Weapon},
    build::BuilderDef,
    costs::{self, Cost},
    dying::DyingDef,
    entity_stats::EntityStatId,
    location::{LocationDef, Solidity},
    morph::MorphTransition,
    repair::{RepairCost, RepairRate, RepairerDef},
    research::{ResearchId, ResearcherDef},
    resource::{
        DepletionPolicy, HarvestData, ResourceCarrierDef, ResourceSourceDef, ResourceStorageDef,
    },
    selection::SelectionDef,
    skills::SkillId,
    splash::SplashDef,
    train::TrainerDef,
    transport::{BoardingPolicy, PassengerConduct, PassengerFate, TransporterDef},
    turret::{TurretFire, TurretMount},
    work::WorkPresence,
};

/// Stable handle for a registered entity type, assigned in registration order by
/// [`ContentRegistry`](crate::registry::ContentRegistry). Cheap to store
/// on an entity and to resolve back to its [`EntityTypeDef`] in O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityTypeId(u32);

impl EntityTypeId {
    /// Wraps a registration index. Registry-internal: handles are minted only by
    /// [`ContentRegistry`](crate::registry::ContentRegistry).
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("entity type count fits in u32"))
    }

    /// The registration index this handle refers to. Identical content registered
    /// in the same order mints identical indices on every peer, which is what
    /// makes it safe to fold into the state checksum.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Content-level blueprint for an entity type (unit, building, resource, …).
///
/// Holds the properties that are identical for every instance of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTypeDef {
    /// Unique type name used to look up this definition in
    /// [`ContentRegistry`](crate::registry::ContentRegistry).
    pub name: String,
    /// The race this type belongs to, by registered race name. `None` means the
    /// type is race-neutral (e.g. resource sources, critters).
    pub race: Option<String>,
    /// Content-declared classification tags (e.g. `building`). Each must be a
    /// registered tag.
    pub tags: BTreeSet<String>,
    /// Requirements for producing an instance — each entry names an entity
    /// type, a tag, or a research, and all must hold (see
    /// [`requirements::met`](crate::requirements::met)).
    pub requires: Vec<String>,

    /// Base value of every stat this type carries, seeded into each instance's
    /// [`StatsComponent`](crate::components::entity_stats::StatsComponent) at spawn. The
    /// built-in stats drive engine behaviour and gate capabilities (an attacker
    /// carries [`EntityStatId::DAMAGE`], a mover [`EntityStatId::SPEED`], …); content may add
    /// custom stats, which are seeded and buffed but otherwise ignored by the engine.
    pub base_stats: BTreeMap<EntityStatId, FixedU64>,
    /// Navigation and footprint properties shared by all instances of this type.
    /// Mandatory for every spawnable type; enforced by
    /// [`ContentRegistry::validate`](crate::registry::ContentRegistry::validate).
    pub location: Option<LocationDef>,
    /// Dying-phase properties. `None` means a destroyed instance is removed
    /// from the world immediately, with no dying phase.
    pub dying: Option<DyingDef>,
    /// Extra damage each hit deals to a target that carries the keyed tag or
    /// whose type name equals the key — the "damage class" side of combat. Added
    /// before the target's armor is subtracted.
    pub bonus_damage_vs: BTreeMap<String, u32>,
    /// The weapon the body itself points, if it has one. It turns to shoot and so
    /// stops to; never defaulted, so a layer added later is never silently in
    /// anyone's reach.
    pub attack: Option<AttackDef>,
    /// The turrets it carries, and where each sits on it.
    pub turrets: Vec<TurretMount>,
    /// How those turrets divide the targets they find for themselves.
    pub turret_fire: TurretFire,
    /// The in-place transitions instances can start, each naming what they
    /// become and on what terms. Empty means instances cannot morph.
    pub morphs: Vec<MorphTransition>,
    /// The navigation layers this type can be attacked on. `None` reads as the
    /// layers it occupies, which is what makes a flier answerable only by
    /// anti-air; declaring it separately is how a thing rooted on one layer is
    /// reached from another.
    pub targetable: Option<LayerMask>,
    /// Activated skills instances of this type can use, by registered id.
    pub skills: Vec<SkillId>,
    /// How instances behave under selection. Every type is selectable, so this is
    /// always present.
    pub selection: SelectionDef,

    /// Price to train or construct one instance. Empty means free.
    pub cost: Cost,
    /// Ticks to train one instance. `None` means the type cannot be trained.
    pub train_time: Option<u32>,
    /// Ticks to construct one instance. `None` means the type cannot be built.
    pub build_time: Option<u32>,
    /// The entity types instances can train. `None` means instances cannot train.
    pub trainer: Option<TrainerDef>,
    /// The passengers instances admit aboard, and on what terms. `None` means
    /// instances cannot carry passengers.
    pub transporter: Option<TransporterDef>,
    /// The researches instances can host. `None` means instances cannot research.
    pub researcher: Option<ResearcherDef>,
    /// The entity types instances can construct. `None` means instances cannot build.
    pub builder: Option<BuilderDef>,
    /// What instances can mend, and on what terms. `None` means instances cannot
    /// repair.
    pub repairer: Option<RepairerDef>,
    /// Multiplier on this type's own production time, giving the time one worker at
    /// `repair_speed` of `1` takes to restore a full pool. `None` reads as `1`.
    pub repair_ratio: Option<FixedU64>,
    /// Resource-source properties. `None` means the entity is not a resource source.
    /// The remaining resource amount is per-instance state, set after spawning.
    pub resource_source: Option<ResourceSourceDef>,
    /// The resource kinds instances can harvest, and how. `None` means the
    /// entity cannot harvest resources.
    pub resource_carrier: Option<ResourceCarrierDef>,
    /// Resource kinds accepted for delivery. `None` means the entity is not a storage.
    pub resource_storage: Option<ResourceStorageDef>,
}

impl EntityTypeDef {
    /// Creates a new definition with the given name.
    ///
    /// Panics if `name` is empty.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(!name.is_empty(), "name must not be empty");

        Self {
            name,
            race: None,
            tags: BTreeSet::new(),
            requires: Vec::new(),
            base_stats: BTreeMap::new(),
            location: None,
            dying: None,
            bonus_damage_vs: BTreeMap::new(),
            attack: None,
            turrets: Vec::new(),
            turret_fire: TurretFire::default(),
            morphs: Vec::new(),
            targetable: None,
            skills: Vec::new(),
            selection: SelectionDef::default(),
            cost: Cost::new(),
            train_time: None,
            build_time: None,
            trainer: None,
            transporter: None,
            researcher: None,
            builder: None,
            repairer: None,
            repair_ratio: None,
            resource_source: None,
            resource_carrier: None,
            resource_storage: None,
        }
    }

    /// The class instances group under for select-all-of-type, defaulting to the
    /// type name when no explicit class was declared.
    pub fn selection_class(&self) -> &str {
        self.selection.class().unwrap_or(&self.name)
    }

    /// The authored base value of `stat`, if this type carries it.
    pub fn base_stat(&self, stat: EntityStatId) -> Option<FixedU64> {
        self.base_stats.get(&stat).copied()
    }

    /// The total bonus damage one hit deals to a target with the given type name
    /// and tags, summed over every matching
    /// [`bonus_damage_vs`](Self::bonus_damage_vs) key.
    pub fn bonus_against(&self, target_type: &str, target_has_tag: impl Fn(&str) -> bool) -> u32 {
        self.bonus_damage_vs
            .iter()
            .filter(|(key, _)| {
                let key = key.as_str();
                key == target_type || target_has_tag(key)
            })
            .map(|(_, &amount)| amount)
            .sum()
    }

    /// Whether instances can attack: the body points a weapon, or they carry a
    /// turret that does.
    pub fn can_attack(&self) -> bool {
        self.attack.is_some() || !self.turrets.is_empty()
    }

    /// Whether instances can move: they carry the [`EntityStatId::SPEED`] stat.
    pub fn can_move(&self) -> bool {
        self.base_stats.contains_key(&EntityStatId::SPEED)
    }

    /// Whether instances have a health pool: they carry the [`EntityStatId::MAX_HEALTH`] stat.
    pub fn has_health(&self) -> bool {
        self.base_stats.contains_key(&EntityStatId::MAX_HEALTH)
    }

    /// Whether instances have an energy pool: they carry the [`EntityStatId::MAX_ENERGY`] stat.
    pub fn has_energy(&self) -> bool {
        self.base_stats.contains_key(&EntityStatId::MAX_ENERGY)
    }

    /// Whether instances can mend other entities.
    pub fn can_repair(&self) -> bool {
        self.repairer.is_some()
    }

    /// Whether instances can carry passengers.
    pub fn can_transport(&self) -> bool {
        self.transporter.is_some()
    }

    /// Ticks to produce one instance, however it is produced. `None` means nothing
    /// produces the type, which also leaves repair no rate to work from.
    pub fn production_time(&self) -> Option<u32> {
        self.build_time.or(self.train_time)
    }

    /// Whether instances can be mended at a rate paced against production. A type
    /// nothing produces gives that pacing nothing to work from; a mender working at
    /// a flat rate does not care.
    pub fn is_production_repairable(&self) -> bool {
        self.has_health() && self.production_time().is_some()
    }

    /// Assigns this type to a race, by registered race name. Race-neutral types
    /// (resource sources, critters) omit this.
    ///
    /// The race must be registered before this type — see
    /// [`ContentRegistry::register`](crate::registry::ContentRegistry::register).
    pub fn with_race(mut self, race: impl Into<String>) -> Self {
        self.race = Some(race.into());
        self
    }

    /// Adds classification tags to this type (see [`tags`](Self::tags)).
    ///
    /// Panics if any tag name is empty.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for tag in tags {
            let tag = tag.into();
            assert!(!tag.is_empty(), "tag names must not be empty");
            self.tags.insert(tag);
        }
        self
    }

    /// Adds production requirements to this type (see [`requires`](Self::requires)).
    ///
    /// Panics if any requirement name is empty.
    pub fn with_requires(mut self, requires: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for name in requires {
            let name = name.into();
            assert!(!name.is_empty(), "requirement names must not be empty");
            self.requires.push(name);
        }
        self
    }

    /// Sets one base stat directly — for a custom (engine-unsupported) stat or a
    /// built-in one. Every base stat is seeded and buffed; the engine reads only
    /// the built-ins.
    pub fn with_stat(mut self, stat: EntityStatId, value: FixedU64) -> Self {
        self.base_stats.insert(stat, value);
        self
    }

    /// Enables movement for this entity type at the given speed (grid units per
    /// tick), with a body of `radius` that resists displacement as `weight`
    /// against what it meets, whose look comes round at `turn_rate` while walking
    /// and at `pivot_rate` while standing (both degrees a tick).
    ///
    /// Panics if `speed` is `0`.
    pub fn with_movement(
        mut self,
        speed: FixedU64,
        radius: FixedU64,
        weight: FixedU64,
        turn_rate: FixedU64,
        pivot_rate: FixedU64,
    ) -> Self {
        self.base_stats.insert(EntityStatId::SPEED, speed);
        self.base_stats.insert(EntityStatId::RADIUS, radius);
        self.base_stats.insert(EntityStatId::WEIGHT, weight);
        self.base_stats.insert(EntityStatId::TURN_RATE, turn_rate);
        self.base_stats.insert(EntityStatId::PIVOT_RATE, pivot_rate);
        self
    }

    /// Enables health for this entity type with the given maximum health points.
    ///
    /// Panics if `max_health` is `0`.
    pub fn with_health(mut self, max_health: u32) -> Self {
        self.base_stats
            .insert(EntityStatId::MAX_HEALTH, FixedU64::from_num(max_health));
        self
    }

    /// Arms this type with the weapon `attack`, which removes `damage` health
    /// points from a target within `range` cells on a cycle of `attack_period`
    /// ticks whose hit lands `damage_point` ticks in, engaging on its own
    /// initiative within `acquire_range`.
    ///
    /// The weapon and its numbers are no use apart, and registration refuses a
    /// type carrying one without the other, so one call carries both. A caller
    /// that sets the numbers itself — scripted content folds every scalar through
    /// [`with_stat`](Self::with_stat) — states the weapon with
    /// [`with_attack_def`](Self::with_attack_def) instead.
    pub fn with_attack(
        self,
        attack: AttackDef,
        damage: u32,
        range: u32,
        acquire_range: u32,
        attack_period: u32,
        damage_point: u32,
    ) -> Self {
        let mut def = self
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(damage))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(range))
            .with_stat(
                EntityStatId::ACQUIRE_RANGE,
                FixedU64::from_num(acquire_range),
            )
            .with_stat(
                EntityStatId::ATTACK_PERIOD,
                FixedU64::from_num(attack_period),
            )
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(damage_point));
        def.attack = Some(attack);
        def
    }

    /// States the weapon the body itself points and nothing else: it reaches
    /// `targets`, its hit travels as `delivery` says and spreads over `splash`.
    /// The numbers it fights by are stats, set separately.
    ///
    /// Panics if `targets` is empty, which would leave the weapon unable to hit
    /// anything at all.
    pub fn with_attack_def(
        mut self,
        targets: impl Into<LayerMask>,
        delivery: Delivery,
        splash: Option<SplashDef>,
    ) -> Self {
        self.attack = Some(AttackDef::new(Weapon::new(targets, delivery, splash)));
        self
    }

    /// Mounts `turrets` on this type, each naming a gun and the patch of the
    /// footprint it sits on.
    pub fn with_turrets(mut self, turrets: impl IntoIterator<Item = TurretMount>) -> Self {
        self.turrets = turrets.into_iter().collect();
        self
    }

    /// Sets how this type's turrets divide the targets they find for themselves
    /// (see [`TurretFire`]).
    pub fn with_turret_fire(mut self, turret_fire: TurretFire) -> Self {
        self.turret_fire = turret_fire;
        self
    }

    /// Sets the flat armor subtracted from each incoming hit (see [`armor`](Self::armor)).
    pub fn with_armor(mut self, armor: u32) -> Self {
        self.base_stats
            .insert(EntityStatId::ARMOR, FixedU64::from_num(armor));
        self
    }

    /// Sets how far instances reveal the map (see [`sight_range`](Self::sight_range)).
    pub fn with_sight_range(mut self, sight_range: u32) -> Self {
        self.base_stats
            .insert(EntityStatId::SIGHT_RANGE, FixedU64::from_num(sight_range));
        self
    }

    /// Gives instances an energy pool of `max` that regenerates `regen` per tick,
    /// for spending on skills.
    pub fn with_energy(mut self, max: u32, regen: FixedU64) -> Self {
        self.base_stats
            .insert(EntityStatId::MAX_ENERGY, FixedU64::from_num(max));
        self.base_stats.insert(EntityStatId::ENERGY_REGEN, regen);
        self
    }

    /// Sets the navigation and footprint properties: the nav-layer occupation,
    /// the footprint size in grid cells, and whether instances claim the cells
    /// they stand on.
    ///
    /// The occupation layers must be registered before this type — see
    /// [`ContentRegistry::register`](crate::registry::ContentRegistry::register).
    ///
    /// Panics if `occupation` is empty or `size` has a zero dimension.
    pub fn with_location(
        mut self,
        occupation: impl Into<LayerMask>,
        size: CellSize,
        solidity: Solidity,
    ) -> Self {
        self.location = Some(LocationDef::new(occupation, size, solidity));
        self
    }

    /// Gives destroyed instances of this type a dying phase of `dying_time`
    /// ticks before they are removed from the world, optionally leaving a
    /// corpse of `corpse_type` behind. The corpse decays through its own dying
    /// phase.
    ///
    /// Panics if `dying_time` is `0` or `corpse_type` is empty.
    pub fn with_dying(mut self, dying_time: u32, corpse_type: Option<&str>) -> Self {
        self.dying = Some(DyingDef::new(dying_time, corpse_type));
        self
    }

    /// Adds per-target damage bonuses, keyed by the target's tag or type name
    /// (see [`bonus_damage_vs`](Self::bonus_damage_vs)).
    ///
    /// Panics if any key is empty.
    pub fn with_bonus_damage_vs(
        mut self,
        bonuses: impl IntoIterator<Item = (impl Into<String>, u32)>,
    ) -> Self {
        for (key, amount) in bonuses {
            let key = key.into();
            assert!(!key.is_empty(), "bonus_damage_vs keys must not be empty");
            self.bonus_damage_vs.insert(key, amount);
        }
        self
    }

    /// Sets the navigation layers this type can be attacked on, overriding the
    /// layers it occupies (see [`targetable`](Self::targetable)).
    ///
    /// Panics if `targetable` is empty, which would make instances invulnerable.
    pub fn with_targetable(mut self, targetable: impl Into<LayerMask>) -> Self {
        let targetable = targetable.into();
        assert!(
            targetable != LayerMask::EMPTY,
            "entity type '{}' is targetable on no layers, so nothing could ever hit it",
            self.name
        );
        self.targetable = Some(targetable);
        self
    }

    /// Adds in-place transitions instances of this type can start (see
    /// [`morphs`](Self::morphs)).
    pub fn with_morphs(mut self, transitions: impl IntoIterator<Item = MorphTransition>) -> Self {
        self.morphs.extend(transitions);
        self
    }

    /// Adds activated skills instances of this type can use (see [`skills`](Self::skills)).
    pub fn with_skills(mut self, skills: impl IntoIterator<Item = SkillId>) -> Self {
        self.skills.extend(skills);
        self
    }

    /// Sets the primary-selection ordering weight and the select-all-of-type
    /// class, which falls back to the type name when `class` is `None`.
    ///
    /// Panics if `class` is empty.
    pub fn with_selection(mut self, priority: u32, class: Option<&str>) -> Self {
        self.selection = SelectionDef::new(priority, class);
        self
    }

    /// Sets the price to produce one instance of this type, from `(kind, amount)`
    /// entries.
    ///
    /// Panics if an entry has an empty resource kind or a zero amount.
    pub fn with_cost(mut self, cost: impl IntoIterator<Item = (impl Into<String>, u32)>) -> Self {
        let cost = costs::cost(cost);

        for (kind, amount) in &cost {
            assert!(!kind.is_empty(), "cost resource kinds must not be empty");
            assert!(*amount > 0, "cost amounts must be greater than 0");
        }

        self.cost = cost;
        self
    }

    /// Makes this type trainable, taking `train_time` ticks per instance.
    ///
    /// Panics if `train_time` is `0`.
    pub fn with_train_time(mut self, train_time: u32) -> Self {
        assert!(train_time > 0, "train_time must be greater than 0");
        self.train_time = Some(train_time);
        self
    }

    /// Makes this type constructible, taking `build_time` ticks.
    ///
    /// Panics if `build_time` is `0`.
    pub fn with_build_time(mut self, build_time: u32) -> Self {
        assert!(build_time > 0, "build_time must be greater than 0");
        self.build_time = Some(build_time);
        self
    }

    /// Allows instances of this type to train units of the `trains` types.
    ///
    /// Panics if `trains` is empty or any entry is empty.
    pub fn with_trainer(mut self, trains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.trainer = Some(TrainerDef::new(trains));
        self
    }

    /// Allows instances of this type to carry passengers matching the
    /// `carries` type names or tags, on the given terms. How much fits aboard
    /// is the `cargo_capacity` stat.
    ///
    /// Panics if `carries` is empty or contains an empty name.
    pub fn with_transporter(
        mut self,
        carries: impl IntoIterator<Item = impl Into<String>>,
        boarding: BoardingPolicy,
        passenger_fate: PassengerFate,
        conduct: PassengerConduct,
    ) -> Self {
        self.transporter = Some(TransporterDef::new(
            carries,
            boarding,
            passenger_fate,
            conduct,
        ));
        self
    }

    /// Allows instances of this type to host the given researches.
    ///
    /// Panics if `researches` is empty.
    pub fn with_researcher(mut self, researches: impl IntoIterator<Item = ResearchId>) -> Self {
        self.researcher = Some(ResearcherDef::new(researches));
        self
    }

    /// Allows instances of this type to construct buildings of the `builds` types,
    /// attending the site as `presence` describes.
    ///
    /// Panics if `builds` is empty or any entry is empty.
    pub fn with_builder(
        mut self,
        builds: impl IntoIterator<Item = impl Into<String>>,
        presence: WorkPresence,
    ) -> Self {
        self.builder = Some(BuilderDef::new(builds, presence));
        self
    }

    /// Allows instances of this type to mend targets carrying the `repairs` tags,
    /// on the given terms.
    ///
    /// Panics if `repairs` is empty or any entry is empty.
    pub fn with_repairer(
        mut self,
        repairs: impl IntoIterator<Item = impl Into<String>>,
        rate: RepairRate,
        presence: WorkPresence,
        self_repair: bool,
        cost: RepairCost,
        patience: Option<u32>,
    ) -> Self {
        self.repairer = Some(RepairerDef::new(
            repairs,
            rate,
            presence,
            self_repair,
            cost,
            patience,
        ));
        self
    }

    /// Sets how long mending this type takes, relative to producing one.
    ///
    /// Panics if `ratio` is not positive.
    pub fn with_repair_ratio(mut self, ratio: FixedU64) -> Self {
        assert!(ratio > FixedU64::ZERO, "repair_ratio must be positive");
        self.repair_ratio = Some(ratio);
        self
    }

    /// Makes instances of this type a resource source yielding `kind`.
    /// `depletion` controls what happens to an instance when it is emptied.
    ///
    /// Panics if `kind` is empty.
    pub fn with_resource_source(
        mut self,
        kind: impl Into<String>,
        depletion: DepletionPolicy,
    ) -> Self {
        self.resource_source = Some(ResourceSourceDef::new(kind, depletion));
        self
    }

    /// Allows instances of this type to harvest the given resource kinds, each
    /// with its own [`HarvestData`].
    ///
    /// Panics if `carries` is empty or any resource kind is empty.
    pub fn with_resource_carrier(
        mut self,
        carries: impl IntoIterator<Item = (impl Into<String>, HarvestData)>,
    ) -> Self {
        self.resource_carrier = Some(ResourceCarrierDef::new(carries));
        self
    }

    /// Makes instances of this type a storage accepting deliveries of the
    /// `accepts` resource kinds.
    ///
    /// Panics if `accepts` is empty or any resource kind is empty.
    pub fn with_resource_storage(
        mut self,
        accepts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.resource_storage = Some(ResourceStorageDef::new(accepts));
        self
    }
}
