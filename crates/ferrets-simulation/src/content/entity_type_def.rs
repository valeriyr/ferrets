//! Definition of a single entity type — the content-level blueprint for spawning.

use std::collections::{BTreeMap, BTreeSet};

use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, nav_size::NavSize};

use crate::{
    components::tags::TagsComponent,
    content::{
        build::BuilderDef,
        dying::DyingDef,
        location::{LocationDef, Solidity},
        projectile::ProjectileId,
        resource::{
            DepletionPolicy, HarvestData, ResourceCarrierDef, ResourceSourceDef, ResourceStorageDef,
        },
        selection::SelectionDef,
        skills::SkillId,
        splash::{SplashDef, SplashShape},
        stats::StatId,
        train::TrainerDef,
    },
    resources::{self, Cost},
};

/// Stable handle for a registered entity type, assigned in registration order by
/// [`ContentRegistry`](crate::content::registry::ContentRegistry). Cheap to store
/// on an entity and to resolve back to its [`EntityTypeDef`] in O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityTypeId(u32);

impl EntityTypeId {
    /// Wraps a registration index. Registry-internal: handles are minted only by
    /// [`ContentRegistry`](crate::content::registry::ContentRegistry).
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("entity type count fits in u32"))
    }

    /// The registration index this handle refers to.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Content-level blueprint for an entity type (unit, building, resource, …).
///
/// Holds the properties that are identical for every instance of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTypeDef {
    /// Unique type name used to look up this definition in
    /// [`ContentRegistry`](crate::content::registry::ContentRegistry).
    pub name: String,
    /// The race this type belongs to, by registered race name. `None` means the
    /// type is race-neutral (e.g. resource sources, critters).
    pub race: Option<String>,
    /// Content-declared classification tags (e.g. `building`). Each must be a
    /// registered tag.
    pub tags: BTreeSet<String>,

    /// Base value of every stat this type carries, seeded into each instance's
    /// [`StatsComponent`](crate::content::stats::StatsComponent) at spawn. The
    /// built-in stats drive engine behaviour and gate capabilities (an attacker
    /// carries [`StatId::DAMAGE`], a mover [`StatId::SPEED`], …); content may add
    /// custom stats, which are seeded and buffed but otherwise ignored by the engine.
    pub base_stats: BTreeMap<StatId, FixedU64>,
    /// Navigation and footprint properties shared by all instances of this type.
    /// Mandatory for every spawnable type; enforced by
    /// [`ContentRegistry::validate`](crate::content::registry::ContentRegistry::validate).
    pub location: Option<LocationDef>,
    /// Dying-phase properties. `None` means a destroyed instance is removed
    /// from the world immediately, with no dying phase.
    pub dying: Option<DyingDef>,
    /// Extra damage each hit deals to a target that carries the keyed tag or
    /// whose type name equals the key — the "damage class" side of combat. Added
    /// before the target's armor is subtracted.
    pub bonus_damage_vs: BTreeMap<String, u32>,
    /// How a hit is delivered. `None` lands the damage in the same tick the attack
    /// cycle reaches its damage point.
    pub projectile: Option<ProjectileId>,
    /// How a hit spreads. `None` damages only the entity that was hit.
    pub splash: Option<SplashDef>,
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
    /// The entity types instances can construct. `None` means instances cannot build.
    pub builder: Option<BuilderDef>,
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
            base_stats: BTreeMap::new(),
            location: None,
            dying: None,
            bonus_damage_vs: BTreeMap::new(),
            projectile: None,
            splash: None,
            skills: Vec::new(),
            selection: SelectionDef::default(),
            cost: Cost::new(),
            train_time: None,
            build_time: None,
            trainer: None,
            builder: None,
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
    pub fn base_stat(&self, stat: StatId) -> Option<FixedU64> {
        self.base_stats.get(&stat).copied()
    }

    /// The total bonus damage one hit deals to a target with the given type name
    /// and tags, summed over every matching
    /// [`bonus_damage_vs`](Self::bonus_damage_vs) key.
    pub fn bonus_against(&self, target_type: &str, target_tags: Option<&TagsComponent>) -> u32 {
        self.bonus_damage_vs
            .iter()
            .filter(|(key, _)| {
                let key = key.as_str();
                key == target_type || target_tags.is_some_and(|tags| tags.contains(key))
            })
            .map(|(_, &amount)| amount)
            .sum()
    }

    /// Whether instances can attack: they carry the [`StatId::DAMAGE`] stat.
    pub fn can_attack(&self) -> bool {
        self.base_stats.contains_key(&StatId::DAMAGE)
    }

    /// Whether instances can move: they carry the [`StatId::SPEED`] stat.
    pub fn can_move(&self) -> bool {
        self.base_stats.contains_key(&StatId::SPEED)
    }

    /// Whether instances can take damage: they carry the [`StatId::MAX_HEALTH`] stat.
    pub fn is_damageable(&self) -> bool {
        self.base_stats.contains_key(&StatId::MAX_HEALTH)
    }

    /// Whether instances have an energy pool: they carry the [`StatId::MAX_ENERGY`] stat.
    pub fn has_energy(&self) -> bool {
        self.base_stats.contains_key(&StatId::MAX_ENERGY)
    }

    /// Assigns this type to a race, by registered race name. Race-neutral types
    /// (resource sources, critters) omit this.
    ///
    /// The race must be registered before this type — see
    /// [`ContentRegistry::register`](crate::content::registry::ContentRegistry::register).
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

    /// Sets one base stat directly — for a custom (engine-unsupported) stat or a
    /// built-in one. Every base stat is seeded and buffed; the engine reads only
    /// the built-ins.
    pub fn with_stat(mut self, stat: StatId, value: FixedU64) -> Self {
        self.base_stats.insert(stat, value);
        self
    }

    /// Enables movement for this entity type at the given speed (grid units per tick).
    ///
    /// Panics if `speed` is `0`.
    pub fn with_movement(mut self, speed: FixedU64) -> Self {
        self.base_stats.insert(StatId::SPEED, speed);
        self
    }

    /// Enables health for this entity type with the given maximum health points.
    ///
    /// Panics if `max_health` is `0`.
    pub fn with_health(mut self, max_health: u32) -> Self {
        self.base_stats
            .insert(StatId::MAX_HEALTH, FixedU64::from_num(max_health));
        self
    }

    /// Enables attacking for this entity type. One hit removes `damage` health
    /// points from a target within `range` grid cells; the full attack cycle is
    /// `attack_period` ticks and the hit lands `damage_point` ticks into it.
    /// `acquire_range` is the distance at which instances notice and engage
    /// enemies on their own initiative.
    pub fn with_attack(
        mut self,
        damage: u32,
        range: u32,
        acquire_range: u32,
        attack_period: u32,
        damage_point: u32,
    ) -> Self {
        self.base_stats
            .insert(StatId::DAMAGE, FixedU64::from_num(damage));
        self.base_stats
            .insert(StatId::ATTACK_RANGE, FixedU64::from_num(range));
        self.base_stats
            .insert(StatId::ACQUIRE_RANGE, FixedU64::from_num(acquire_range));
        self.base_stats
            .insert(StatId::ATTACK_PERIOD, FixedU64::from_num(attack_period));
        self.base_stats
            .insert(StatId::DAMAGE_POINT, FixedU64::from_num(damage_point));
        self
    }

    /// Sets the flat armor subtracted from each incoming hit (see [`armor`](Self::armor)).
    pub fn with_armor(mut self, armor: u32) -> Self {
        self.base_stats
            .insert(StatId::ARMOR, FixedU64::from_num(armor));
        self
    }

    /// Sets how far instances reveal the map (see [`sight_range`](Self::sight_range)).
    pub fn with_sight_range(mut self, sight_range: u32) -> Self {
        self.base_stats
            .insert(StatId::SIGHT_RANGE, FixedU64::from_num(sight_range));
        self
    }

    /// Gives instances an energy pool of `max` that regenerates `regen` per tick,
    /// for spending on skills.
    pub fn with_energy(mut self, max: u32, regen: FixedU64) -> Self {
        self.base_stats
            .insert(StatId::MAX_ENERGY, FixedU64::from_num(max));
        self.base_stats.insert(StatId::ENERGY_REGEN, regen);
        self
    }

    /// Sets the navigation and footprint properties: the nav-layer occupation,
    /// the footprint size in grid cells, and whether instances claim the cells
    /// they stand on.
    ///
    /// The occupation layers must be registered before this type — see
    /// [`ContentRegistry::register`](crate::content::registry::ContentRegistry::register).
    ///
    /// Panics if `occupation` is empty or `size` has a zero dimension.
    pub fn with_location(
        mut self,
        occupation: impl Into<LayerMask>,
        size: NavSize,
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

    /// Delivers this type's hits as the registered projectile kind, instead of
    /// landing them at the damage point.
    ///
    /// The projectile must be registered before this type — see
    /// [`ContentRegistry::register`](crate::content::registry::ContentRegistry::register).
    pub fn with_projectile(mut self, projectile: ProjectileId) -> Self {
        self.projectile = Some(projectile);
        self
    }

    /// Spreads this type's hits over an area: `bands` are `(radius, fraction)`
    /// pairs in increasing radius order, `layers` the navigation layers the blast
    /// reaches, and `friendly_fire` whether it also catches own and allied entities.
    ///
    /// Panics if `bands` is empty, its radii are not strictly increasing, or
    /// `layers` is empty.
    pub fn with_splash(
        mut self,
        shape: SplashShape,
        bands: Vec<(u32, FixedU64)>,
        layers: impl Into<LayerMask>,
        friendly_fire: bool,
    ) -> Self {
        self.splash = Some(SplashDef::new(shape, bands, layers, friendly_fire));
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
        let cost = resources::cost(cost);

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

    /// Allows instances of this type to construct buildings of the `builds` types.
    ///
    /// Panics if `builds` is empty or any entry is empty.
    pub fn with_builder(mut self, builds: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.builder = Some(BuilderDef::new(builds));
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
