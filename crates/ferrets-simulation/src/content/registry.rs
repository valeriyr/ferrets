//! Registry of all content-defined entity types and resource kinds.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

use super::entity_type_def::{EntityTypeDef, EntityTypeId};
use crate::components::buffs::{BuffDef, BuffId};
use crate::components::skills::{SkillDef, SkillId};
use crate::components::stats::{BUILTIN_STATS, StatId};
use crate::components::tags;

/// Stores every [`EntityTypeDef`], indexed by [`EntityTypeId`] and looked up by
/// type name, as well as all the other registered content.
///
/// Everything is held in ordered containers, so iteration over the registry is
/// deterministic.
#[derive(Resource)]
pub struct ContentRegistry {
    /// All type definitions, indexed by [`EntityTypeId`] (registration order).
    defs: Vec<EntityTypeDef>,
    /// Type name → handle, for name lookups and name-sorted iteration.
    defs_by_name: BTreeMap<String, EntityTypeId>,
    resources: BTreeSet<String>,
    races: BTreeSet<String>,
    tags: BTreeSet<String>,
    layers: BTreeMap<String, LayerId>,
    terrains: BTreeMap<String, LayerMask>,
    stats: BTreeMap<String, StatId>,
    buffs: BTreeMap<String, BuffId>,
    buff_defs: Vec<BuffDef>,
    skills: BTreeMap<String, SkillId>,
    skill_defs: Vec<SkillDef>,
}

impl Default for ContentRegistry {
    /// A fresh registry already carries the engine's reserved tags.
    fn default() -> Self {
        Self {
            defs: Vec::new(),
            defs_by_name: BTreeMap::new(),
            resources: BTreeSet::new(),
            races: BTreeSet::new(),
            tags: BTreeSet::from([tags::BUILDING.to_string()]),
            layers: BTreeMap::new(),
            terrains: BTreeMap::new(),
            stats: BUILTIN_STATS
                .iter()
                .map(|builtin| (builtin.name.to_string(), builtin.id))
                .collect(),
            buffs: BTreeMap::new(),
            buff_defs: Vec::new(),
            skills: BTreeMap::new(),
            skill_defs: Vec::new(),
        }
    }
}

impl ContentRegistry {
    /// Registers an entity type definition.
    ///
    /// Validates everything intrinsic to the definition or that must form an
    /// acyclic hierarchy — so resource kinds it references and any corpse type it
    /// leaves must be registered first, and corpse cycles stay unconstructible.
    /// Production catalogues (trained/built types) are *not* checked here because
    /// they may legitimately reference each other cyclically (a town hall trains a
    /// worker that builds the town hall); they are validated by [`validate`] once
    /// all content is registered. Registration is final — a type cannot be
    /// replaced — so a validated definition stays consistent.
    ///
    /// [`validate`]: Self::validate
    ///
    /// Panics if a type with the same name is already registered, or if the
    /// definition has no location, belongs to an unregistered race, references an
    /// unregistered resource kind or tag, carries a skill with an energy cost but no
    /// energy pool, or leaves a corpse type that is unregistered, has no dying phase,
    /// or defines live-gameplay data.
    pub fn register(&mut self, def: EntityTypeDef) {
        assert!(
            !self.defs_by_name.contains_key(&def.name),
            "entity type '{}' is already registered",
            def.name
        );

        self.validate_location(&def);
        self.validate_race(&def);
        self.validate_resource_kinds(&def);
        self.validate_tags(&def);
        self.validate_layers(&def);
        self.validate_corpse(&def);
        self.validate_stats(&def);
        self.validate_skills(&def);

        let id = EntityTypeId::from_index(self.defs.len());
        self.defs_by_name.insert(def.name.clone(), id);
        self.defs.push(def);
    }

    /// Validates the production catalogues of all registered types. Call once
    /// after every type has been registered.
    ///
    /// These references (trained and built types) may form cycles, so they cannot
    /// be checked at registration time; this pass checks them against the complete
    /// registry, in any registration order.
    ///
    /// Panics if any type trains a type that is not a registered trainable type,
    /// or builds a type that is not a registered constructible type.
    pub fn validate(&self) {
        for def in &self.defs {
            self.validate_trains(def);
            self.validate_builds(def);
        }
    }

    /// Returns the definition for the given type name, or `None` if not registered.
    pub fn entity(&self, name: &str) -> Option<&EntityTypeDef> {
        self.defs_by_name
            .get(name)
            .map(|&id| &self.defs[id.index()])
    }

    /// Returns the definition for the given handle.
    pub fn def(&self, id: EntityTypeId) -> &EntityTypeDef {
        &self.defs[id.index()]
    }

    /// Returns the handle for the given type name, or `None` if not registered.
    pub fn type_id(&self, name: &str) -> Option<EntityTypeId> {
        self.defs_by_name.get(name).copied()
    }

    /// Returns every registered entity type definition, in ascending name order.
    pub fn entities(&self) -> impl Iterator<Item = &EntityTypeDef> {
        self.defs_by_name.values().map(|&id| &self.defs[id.index()])
    }

    /// Returns the registered resource kinds, in ascending order.
    pub fn resources(&self) -> impl Iterator<Item = &str> {
        self.resources.iter().map(String::as_str)
    }

    /// Registers a resource kind (gold, wood, …).
    ///
    /// Panics if `kind` is empty.
    pub fn register_resource(&mut self, kind: impl Into<String>) {
        let kind = kind.into();
        assert!(!kind.is_empty(), "kind must not be empty");
        self.resources.insert(kind);
    }

    /// Returns `true` if `kind` is a registered resource kind.
    pub fn has_resource(&self, kind: &str) -> bool {
        self.resources.contains(kind)
    }

    /// Registers a race (human, orc, …).
    ///
    /// Panics if `name` is empty.
    pub fn register_race(&mut self, name: impl Into<String>) {
        let name = name.into();
        assert!(!name.is_empty(), "race name must not be empty");
        self.races.insert(name);
    }

    /// Returns `true` if `name` is a registered race.
    pub fn has_race(&self, name: &str) -> bool {
        self.races.contains(name)
    }

    /// Registers a classification tag (building, …).
    ///
    /// Panics if `tag` is empty.
    pub fn register_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        assert!(!tag.is_empty(), "tag must not be empty");
        self.tags.insert(tag);
    }

    /// Returns `true` if `tag` is a registered tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// Registers a navigation layer (ground, air, …) and returns its assigned
    /// [`LayerId`].
    ///
    /// Ids are assigned in registration order, so identical content registered
    /// in the same order resolves to identical ids everywhere. Re-registering a
    /// name returns its existing id.
    ///
    /// Panics if `name` is empty or all layer ids are already assigned.
    pub fn register_layer(&mut self, name: impl Into<String>) -> LayerId {
        let name = name.into();
        assert!(!name.is_empty(), "layer name must not be empty");

        if let Some(&id) = self.layers.get(&name) {
            return id;
        }

        let bit = u32::try_from(self.layers.len()).unwrap();
        assert!(
            bit < u32::BITS,
            "all {} layer ids are already assigned",
            u32::BITS
        );
        let id = LayerId::new(1 << bit);
        self.layers.insert(name, id);
        id
    }

    /// Returns `true` if `name` is a registered navigation layer.
    pub fn has_layer(&self, name: &str) -> bool {
        self.layers.contains_key(name)
    }

    /// Returns the id assigned to the given layer name, or `None` if not
    /// registered.
    pub fn layer(&self, name: &str) -> Option<LayerId> {
        self.layers.get(name).copied()
    }

    /// Returns every registered navigation layer with its assigned id, in
    /// ascending name order.
    pub fn layers(&self) -> impl Iterator<Item = (&str, LayerId)> {
        self.layers.iter().map(|(name, &id)| (name.as_str(), id))
    }

    /// Registers a stat (health, damage, …) and returns its assigned [`StatId`].
    ///
    /// Ids are assigned in registration order. The built-in stats are
    /// pre-registered first, so their ids are the [`StatId`] constants, and
    /// content-declared stats follow. Re-registering a name returns its
    /// existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_stat(&mut self, name: impl Into<String>) -> StatId {
        let name = name.into();
        assert!(!name.is_empty(), "stat name must not be empty");

        if let Some(&id) = self.stats.get(&name) {
            return id;
        }

        let id = StatId::from_index(self.stats.len());
        self.stats.insert(name, id);
        id
    }

    /// Returns `true` if `name` is a registered stat.
    pub fn has_stat(&self, name: &str) -> bool {
        self.stats.contains_key(name)
    }

    /// Returns the [`StatId`] for the given stat name, or `None` if not registered.
    pub fn stat(&self, name: &str) -> Option<StatId> {
        self.stats.get(name).copied()
    }

    /// Registers a buff definition by name and returns its assigned [`BuffId`].
    /// Ids are assigned in registration order, so identical content registered in
    /// the same order resolves to identical ids everywhere. Re-registering a name
    /// keeps the first definition and returns its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_buff(&mut self, name: impl Into<String>, buff: BuffDef) -> BuffId {
        let name = name.into();
        assert!(!name.is_empty(), "buff name must not be empty");

        if let Some(&id) = self.buffs.get(&name) {
            return id;
        }

        let id = BuffId::from_index(self.buff_defs.len());
        self.buffs.insert(name, id);
        self.buff_defs.push(buff);
        id
    }

    /// Returns `true` if `name` is a registered buff.
    pub fn has_buff(&self, name: &str) -> bool {
        self.buffs.contains_key(name)
    }

    /// Returns the [`BuffId`] for the given buff name, or `None` if not registered.
    pub fn buff(&self, name: &str) -> Option<BuffId> {
        self.buffs.get(name).copied()
    }

    /// Returns the name the given buff is registered under, or `None` if the
    /// handle did not come from this registry.
    pub fn buff_name(&self, id: BuffId) -> Option<&str> {
        self.buffs
            .iter()
            .find(|&(_, &buff)| buff == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the buff definition for the given handle.
    pub fn buff_def(&self, id: BuffId) -> &BuffDef {
        &self.buff_defs[id.index()]
    }

    /// Registers a skill definition by name and returns its assigned [`SkillId`].
    /// Ids are assigned in registration order, so identical content registered in
    /// the same order resolves to identical ids everywhere. Re-registering a name
    /// keeps the first definition and returns its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_skill(&mut self, name: impl Into<String>, skill: SkillDef) -> SkillId {
        let name = name.into();
        assert!(!name.is_empty(), "skill name must not be empty");

        if let Some(&id) = self.skills.get(&name) {
            return id;
        }

        let id = SkillId::from_index(self.skill_defs.len());
        self.skills.insert(name, id);
        self.skill_defs.push(skill);
        id
    }

    /// Returns `true` if `name` is a registered skill.
    pub fn has_skill(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }

    /// Returns the [`SkillId`] for the given skill name, or `None` if not registered.
    pub fn skill(&self, name: &str) -> Option<SkillId> {
        self.skills.get(name).copied()
    }

    /// Returns the name the given skill is registered under, or `None` if the
    /// handle did not come from this registry.
    pub fn skill_name(&self, id: SkillId) -> Option<&str> {
        self.skills
            .iter()
            .find(|&(_, &skill)| skill == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the skill definition for the given handle.
    pub fn skill_def(&self, id: SkillId) -> &SkillDef {
        &self.skill_defs[id.index()]
    }

    /// Registers a terrain type (grass, water, …): a name and the mask of
    /// navigation layers passable on cells of that terrain. An empty mask means
    /// the terrain is impassable on every layer.
    ///
    /// The passable layers must be registered first.
    ///
    /// Panics if `name` is empty, the terrain is already registered, or the
    /// mask includes an unregistered layer.
    pub fn register_terrain(&mut self, name: impl Into<String>, passable: impl Into<LayerMask>) {
        let name = name.into();
        let passable = passable.into();

        assert!(!name.is_empty(), "terrain name must not be empty");
        assert!(
            !self.terrains.contains_key(&name),
            "terrain '{name}' is already registered"
        );
        let unregistered = passable & !self.registered_layers();
        assert!(
            unregistered == LayerMask::EMPTY,
            "terrain '{name}' passes unregistered layers {unregistered}"
        );

        self.terrains.insert(name, passable);
    }

    /// Returns `true` if `name` is a registered terrain type.
    pub fn has_terrain(&self, name: &str) -> bool {
        self.terrains.contains_key(name)
    }

    /// Returns the mask of layers passable on the given terrain, or `None` if
    /// not registered.
    pub fn terrain(&self, name: &str) -> Option<LayerMask> {
        self.terrains.get(name).copied()
    }

    /// Returns the mask of every registered navigation layer.
    pub fn registered_layers(&self) -> LayerMask {
        self.layers
            .values()
            .fold(LayerMask::EMPTY, |mask, &id| mask | id)
    }

    /// Checks that the definition has the mandatory location properties.
    fn validate_location(&self, def: &EntityTypeDef) {
        assert!(
            def.location.is_some(),
            "entity type '{}' has no location",
            def.name
        );
    }

    /// Checks that every skill the type carries can be paid for: a skill with an
    /// energy cost needs the pool to spend from, so the type must declare
    /// [`StatId::MAX_ENERGY`].
    fn validate_skills(&self, def: &EntityTypeDef) {
        for &skill in &def.skills {
            if self.skill_def(skill).energy_cost > FixedU64::ZERO {
                assert!(
                    def.has_energy(),
                    "entity type '{}' has skill '{}' with an energy cost but no max_energy stat",
                    def.name,
                    self.skill_name(skill).unwrap_or("<unregistered>"),
                );
            }
        }
    }

    /// Checks the engine's built-in stats: a declared pool or speed is positive (a
    /// zero would be meaningless); a stat the engine reads as a whole number is at
    /// least its floor; an attacker — one carrying the [`StatId::DAMAGE`] stat —
    /// also carries the rest of its weapon; and the hit lands within the attack
    /// cycle (`damage_point <= attack_period`).
    /// Content's own custom stats are engine-transparent and not checked here.
    fn validate_stats(&self, def: &EntityTypeDef) {
        // Declaring any of these at zero says nothing an omitted stat would not.
        for stat in [StatId::MAX_HEALTH, StatId::SPEED, StatId::MAX_ENERGY] {
            if let Some(value) = def.base_stat(stat) {
                assert!(
                    value > FixedU64::ZERO,
                    "entity type '{}' has a non-positive {} stat",
                    def.name,
                    BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        // A floored stat is one the engine reads as a whole number, so an authored
        // value below the floor truncates to something its consumer can never
        // satisfy — and an entity that is never buffed never reaches the fold that
        // would raise it. Driven off the floor table so the two cannot disagree.
        for builtin in &BUILTIN_STATS {
            if builtin.floor == FixedU64::ZERO {
                continue;
            }
            if let Some(value) = def.base_stat(builtin.id) {
                assert!(
                    value >= builtin.floor,
                    "entity type '{}' has {} below its minimum of {}",
                    def.name,
                    builtin.name,
                    builtin.floor,
                );
            }
        }

        if def.can_attack() {
            for stat in [
                StatId::ATTACK_RANGE,
                StatId::ACQUIRE_RANGE,
                StatId::ATTACK_PERIOD,
                StatId::DAMAGE_POINT,
            ] {
                assert!(
                    def.base_stat(stat).is_some(),
                    "entity type '{}' carries the damage stat but is missing {}",
                    def.name,
                    BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        if let (Some(period), Some(damage_point)) = (
            def.base_stat(StatId::ATTACK_PERIOD),
            def.base_stat(StatId::DAMAGE_POINT),
        ) {
            assert!(
                damage_point <= period,
                "entity type '{}' has a damage_point beyond its attack_period",
                def.name
            );
        }
    }

    /// Checks that the definition's race, if any, is registered.
    fn validate_race(&self, def: &EntityTypeDef) {
        if let Some(race) = &def.race {
            assert!(
                self.has_race(race),
                "entity type '{}' belongs to unregistered race '{race}'",
                def.name
            );
        }
    }

    /// Checks that every resource kind the definition references is registered.
    fn validate_resource_kinds(&self, def: &EntityTypeDef) {
        let check_kind = |kind: &str, role: &str| {
            assert!(
                self.has_resource(kind),
                "entity type '{}' references unregistered resource kind '{kind}' in its {role}",
                def.name
            );
        };

        for kind in def.cost.keys() {
            check_kind(kind, "cost");
        }
        if let Some(source) = &def.resource_source {
            check_kind(source.kind(), "resource source");
        }
        if let Some(carrier) = &def.resource_carrier {
            for kind in carrier.kinds() {
                check_kind(kind, "resource carrier");
            }
        }
        if let Some(storage) = &def.resource_storage {
            for kind in storage.kinds() {
                check_kind(kind, "resource storage");
            }
        }
    }

    /// Checks that every tag the definition carries is registered.
    fn validate_tags(&self, def: &EntityTypeDef) {
        for tag in &def.tags {
            assert!(
                self.has_tag(tag),
                "entity type '{}' references unregistered tag '{tag}'",
                def.name
            );
        }
    }

    /// Checks that the definition occupies only registered navigation layers.
    fn validate_layers(&self, def: &EntityTypeDef) {
        let Some(location) = &def.location else {
            return;
        };

        let unregistered = location.occupation() & !self.registered_layers();
        assert!(
            unregistered == LayerMask::EMPTY,
            "entity type '{}' occupies unregistered layers {unregistered}",
            def.name
        );
    }

    /// Checks that every type in the definition's train catalogue is a
    /// registered trainable type.
    fn validate_trains(&self, def: &EntityTypeDef) {
        let Some(trainer) = &def.trainer else { return };

        for type_name in trainer.trains() {
            let trainable = self
                .entity(type_name)
                .is_some_and(|trained| trained.train_time.is_some());
            assert!(
                trainable,
                "entity type '{}' trains '{type_name}', which is not a registered trainable type",
                def.name
            );
        }
    }

    /// Checks that the definition's corpse type is registered, has a dying
    /// phase, and defines only corpse-compatible data.
    ///
    /// The decay chain needs no termination check: a corpse type is validated
    /// when it is registered, which must happen before anything can reference
    /// it, so by induction every chain bottoms out and no cycle can form.
    fn validate_corpse(&self, def: &EntityTypeDef) {
        let Some(corpse_type) = def.dying.as_ref().and_then(|dying| dying.corpse_type()) else {
            return;
        };

        assert!(
            self.entity(corpse_type).is_some(),
            "entity type '{}' leaves an unregistered corpse type '{corpse_type}'",
            def.name
        );
        assert!(
            self.entity(corpse_type).unwrap().dying.is_some(),
            "entity type '{}' leaves a corpse type '{corpse_type}' that has no dying phase",
            def.name
        );
        self.validate_corpse_compatible(def, corpse_type);
    }

    /// Checks that a corpse type defines only data remains can use: identity,
    /// footprint, occupation, and a dying phase. Corpses are spawned directly
    /// into the dying state, so any other definition data would be silently
    /// ignored.
    ///
    /// Implemented as an equality check against a minimal definition carrying
    /// only the allowed data, so fields added to [`EntityTypeDef`] later are
    /// corpse-incompatible by default.
    fn validate_corpse_compatible(&self, user: &EntityTypeDef, corpse_type: &str) {
        let corpse = self.entity(corpse_type).expect("corpse type is registered");

        let mut allowed = EntityTypeDef::new(corpse.name.clone());
        allowed.race = corpse.race.clone();
        allowed.location = corpse.location;
        allowed.dying = corpse.dying.clone();

        assert_eq!(
            *corpse, allowed,
            "entity type '{}' uses '{corpse_type}' as a corpse type, but '{corpse_type}' defines live-gameplay data that remains never use",
            user.name
        );
    }

    /// Checks that every type in the definition's build catalogue is a
    /// registered constructible type.
    fn validate_builds(&self, def: &EntityTypeDef) {
        let Some(builder) = &def.builder else { return };

        for type_name in builder.builds() {
            let constructible = self
                .entity(type_name)
                .is_some_and(|built| built.build_time.is_some());
            assert!(
                constructible,
                "entity type '{}' builds '{type_name}', which is not a registered constructible type",
                def.name
            );
        }
    }
}
