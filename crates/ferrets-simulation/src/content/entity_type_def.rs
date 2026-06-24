//! Definition of a single entity type — the content-level blueprint for spawning.

use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_mask::LayerMask, nav_size::NavSize};

use crate::{
    components::{
        attack::AttackStaticData,
        build::BuilderStaticData,
        dying::DyingStaticData,
        health::HealthStaticData,
        location::{LocationStaticData, Solidity},
        movement::MoveStaticData,
        resource::{
            DepletionPolicy, HarvestData, ResourceCarrierStaticData, ResourceSourceStaticData,
            ResourceStorageStaticData,
        },
        train::TrainStaticData,
    },
    resources::Cost,
};

/// Content-level blueprint for an entity type (unit, building, resource, …).
///
/// Holds the static data components that are identical for every instance of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTypeDef {
    /// Unique type name used to look up this definition in [`ContentRegistry`].
    pub name: String,
    /// The race this type belongs to, by registered race name. `None` means the
    /// type is race-neutral (e.g. resource sources, critters).
    pub race: Option<String>,
    /// Navigation and footprint properties shared by all instances of this type.
    /// Mandatory for every spawnable type; enforced by
    /// [`ContentRegistry::validate`](crate::content::registry::ContentRegistry::validate).
    pub location: Option<LocationStaticData>,
    /// Movement properties. `None` means the entity cannot move.
    pub movement: Option<MoveStaticData>,
    /// Health properties. `None` means the entity cannot take damage.
    pub health: Option<HealthStaticData>,
    /// Dying-phase properties. `None` means a destroyed instance is removed
    /// from the world immediately, with no dying phase.
    pub dying: Option<DyingStaticData>,
    /// Combat properties. `None` means the entity cannot attack.
    pub attack: Option<AttackStaticData>,
    /// Price to train or construct one instance. Empty means free.
    pub cost: Cost,
    /// Ticks to train one instance. `None` means the type cannot be trained.
    pub train_time: Option<u32>,
    /// Ticks to construct one instance. `None` means the type cannot be built.
    pub build_time: Option<u32>,
    /// The entity types instances can train. `None` means instances cannot train.
    pub trainer: Option<TrainStaticData>,
    /// The entity types instances can construct. `None` means instances cannot build.
    pub builder: Option<BuilderStaticData>,
    /// Resource-source properties. `None` means the entity is not a resource source.
    /// The remaining resource amount is per-instance state, set after spawning.
    pub resource_source: Option<ResourceSourceStaticData>,
    /// The resource kinds instances can harvest, and how. `None` means the
    /// entity cannot harvest resources.
    pub resource_carrier: Option<ResourceCarrierStaticData>,
    /// Resource kinds accepted for delivery. `None` means the entity is not a storage.
    pub resource_storage: Option<ResourceStorageStaticData>,
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
            location: None,
            movement: None,
            health: None,
            dying: None,
            attack: None,
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

    /// Assigns this type to a race, by registered race name. Race-neutral types
    /// (resource sources, critters) omit this.
    ///
    /// The race must be registered before this type — see
    /// [`ContentRegistry::register`](crate::content::registry::ContentRegistry::register).
    pub fn with_race(mut self, race: impl Into<String>) -> Self {
        self.race = Some(race.into());
        self
    }

    /// Sets the navigation and footprint properties: the nav-layer occupation,
    /// the footprint size in grid cells, and whether instances claim the cells
    /// they stand on.
    ///
    /// Panics if `occupation` is empty or `size` has a zero dimension.
    pub fn with_location(
        mut self,
        occupation: impl Into<LayerMask>,
        size: NavSize,
        solidity: Solidity,
    ) -> Self {
        self.location = Some(LocationStaticData::new(occupation, size, solidity));
        self
    }

    /// Enables movement for this entity type at the given speed (grid units per tick).
    ///
    /// Panics if `speed` is `0`.
    pub fn with_movement(mut self, speed: FixedU64) -> Self {
        self.movement = Some(MoveStaticData::new(speed));
        self
    }

    /// Enables health for this entity type with the given maximum health points.
    ///
    /// Panics if `max_health` is `0`.
    pub fn with_health(mut self, max_health: u32) -> Self {
        self.health = Some(HealthStaticData::new(max_health));
        self
    }

    /// Gives destroyed instances of this type a dying phase of `dying_time`
    /// ticks before they are removed from the world, optionally leaving a
    /// corpse of `corpse_type` behind. The corpse decays through its own dying
    /// phase.
    ///
    /// Panics if `dying_time` is `0` or `corpse_type` is empty.
    pub fn with_dying(mut self, dying_time: u32, corpse_type: Option<&str>) -> Self {
        self.dying = Some(DyingStaticData::new(dying_time, corpse_type));
        self
    }

    /// Enables attacking for this entity type. One hit removes `damage` health
    /// points from a target within `range` grid cells; a swing lands after
    /// `aiming` ticks and the next one starts after `reloading` more.
    ///
    /// Panics if `aiming` or `reloading` is `0`.
    pub fn with_attack(mut self, damage: u32, range: u32, aiming: u32, reloading: u32) -> Self {
        self.attack = Some(AttackStaticData::new(damage, range, aiming, reloading));
        self
    }

    /// Sets the price to produce one instance of this type, from `(kind, amount)`
    /// entries.
    ///
    /// Panics if an entry has an empty resource kind or a zero amount.
    pub fn with_cost(mut self, cost: impl IntoIterator<Item = (impl Into<String>, u32)>) -> Self {
        let cost: Cost = cost
            .into_iter()
            .map(|(kind, amount)| (kind.into(), amount))
            .collect();

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
        self.trainer = Some(TrainStaticData::new(trains));
        self
    }

    /// Allows instances of this type to construct buildings of the `builds` types.
    ///
    /// Panics if `builds` is empty or any entry is empty.
    pub fn with_builder(mut self, builds: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.builder = Some(BuilderStaticData::new(builds));
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
        self.resource_source = Some(ResourceSourceStaticData::new(kind, depletion));
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
        self.resource_carrier = Some(ResourceCarrierStaticData::new(carries));
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
        self.resource_storage = Some(ResourceStorageStaticData::new(accepts));
        self
    }
}
