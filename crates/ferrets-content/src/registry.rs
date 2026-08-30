//! Registry of all content-defined entity types and resource kinds.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

use crate::{
    attack::{AttackDef, Delivery, Weapon},
    entity_buffs::{EntityBuffDef, EntityBuffId},
    entity_stats::{ENTITY_BUILTIN_STATS, EntityStatId},
    entity_type_def::{EntityTypeDef, EntityTypeId},
    morph::MorphTime,
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    player_stats::{PLAYER_BUILTIN_STATS, PlayerStatId},
    projectile::{Aim, ProjectileDef, ProjectileId},
    repair::RepairCost,
    research::{ResearchDef, ResearchId},
    skills::{EntityCastCost, EntityCastEffect, PlayerCastEffect, SkillCaster, SkillDef, SkillId},
    tags,
    turret::{TurretDef, TurretId, WeaponConduct},
};

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
    entity_stats: BTreeMap<String, EntityStatId>,
    player_stats: BTreeMap<String, PlayerStatId>,
    entity_buffs: BTreeMap<String, EntityBuffId>,
    entity_buff_defs: Vec<EntityBuffDef>,
    player_buffs: BTreeMap<String, PlayerBuffId>,
    player_buff_defs: Vec<PlayerBuffDef>,
    skills: BTreeMap<String, SkillId>,
    skill_defs: Vec<SkillDef>,
    researches: BTreeMap<String, ResearchId>,
    research_defs: Vec<ResearchDef>,
    projectiles: BTreeMap<String, ProjectileId>,
    projectile_defs: Vec<ProjectileDef>,
    turrets: BTreeMap<String, TurretId>,
    turret_defs: Vec<TurretDef>,
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
            entity_stats: ENTITY_BUILTIN_STATS
                .iter()
                .map(|builtin| (builtin.name.to_string(), builtin.id))
                .collect(),
            player_stats: PLAYER_BUILTIN_STATS
                .iter()
                .map(|builtin| (builtin.name.to_string(), builtin.id))
                .collect(),
            entity_buffs: BTreeMap::new(),
            entity_buff_defs: Vec::new(),
            player_buffs: BTreeMap::new(),
            player_buff_defs: Vec::new(),
            skills: BTreeMap::new(),
            skill_defs: Vec::new(),
            researches: BTreeMap::new(),
            research_defs: Vec::new(),
            projectiles: BTreeMap::new(),
            projectile_defs: Vec::new(),
            turrets: BTreeMap::new(),
            turret_defs: Vec::new(),
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
    /// energy pool, delivers a hit without a damage stat, splashes onto unregistered
    /// layers, or leaves a corpse type that is unregistered, has no dying phase,
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
        self.validate_researcher(&def);
        self.validate_delivery(&def);
        self.validate_repair(&def);
        self.validate_build(&def);
        self.validate_harvest(&def);
        self.validate_transport(&def);

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
    /// builds a type that is not a registered constructible type, carries a
    /// requirement (on the type itself, a research, or a skill) that does not
    /// resolve to exactly one of: a registered entity type or tag, or a
    /// registered research, deals bonus damage to a name that is neither a
    /// registered entity type nor a registered tag, moves on a combination of
    /// layers no registered terrain passes, or offers a morph transition whose
    /// destination is unregistered or an odd footprint span away, whose
    /// requirements do not resolve, or whose costs draw from a pool the type does
    /// not have.
    pub fn validate(&self) {
        for def in &self.defs {
            self.validate_trains(def);
            self.validate_builds(def);
            self.validate_carries(def);
            self.validate_requires(&format!("entity type '{}'", def.name), &def.requires);
            self.validate_bonus_damage_vs(def);
            self.validate_traversable(def);
            self.validate_morphs(def);
        }
        for (name, &id) in &self.researches {
            self.validate_requires(
                &format!("research '{name}'"),
                &self.research_defs[id.index()].requires,
            );
        }
        for (name, &id) in &self.skills {
            self.validate_requires(
                &format!("skill '{name}'"),
                &self.skill_defs[id.index()].requires,
            );
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

    /// Registers an entity stat (health, damage, …) and returns its assigned
    /// [`EntityStatId`].
    ///
    /// Ids are assigned in registration order. The built-in stats are
    /// pre-registered first, so their ids are the [`EntityStatId`] constants, and
    /// content-declared stats follow. Re-registering a name returns its
    /// existing id.
    ///
    /// Panics if `name` is empty or already names a player stat — the two
    /// vocabularies are separate, but one name meaning both would leave content
    /// ambiguous to its readers.
    pub fn register_entity_stat(&mut self, name: impl Into<String>) -> EntityStatId {
        let name = name.into();
        assert!(!name.is_empty(), "stat name must not be empty");
        assert!(
            !self.player_stats.contains_key(&name),
            "'{name}' is already registered as a player stat"
        );

        if let Some(&id) = self.entity_stats.get(&name) {
            return id;
        }

        let id = EntityStatId::from_index(self.entity_stats.len());
        self.entity_stats.insert(name, id);
        id
    }

    /// Returns `true` if `name` is a registered entity stat.
    pub fn has_entity_stat(&self, name: &str) -> bool {
        self.entity_stats.contains_key(name)
    }

    /// Returns the [`EntityStatId`] for the given stat name, or `None` if not registered.
    pub fn entity_stat(&self, name: &str) -> Option<EntityStatId> {
        self.entity_stats.get(name).copied()
    }

    /// Registers a player stat (max_supply, …) and returns its assigned
    /// [`PlayerStatId`].
    ///
    /// Ids are assigned in registration order. The built-in player stats are
    /// pre-registered first, so their ids are the [`PlayerStatId`] constants, and
    /// content-declared player stats follow. Re-registering a name returns its
    /// existing id.
    ///
    /// Panics if `name` is empty or already names an entity stat.
    pub fn register_player_stat(&mut self, name: impl Into<String>) -> PlayerStatId {
        let name = name.into();
        assert!(!name.is_empty(), "player stat name must not be empty");
        assert!(
            !self.entity_stats.contains_key(&name),
            "'{name}' is already registered as an entity stat"
        );

        if let Some(&id) = self.player_stats.get(&name) {
            return id;
        }

        let id = PlayerStatId::from_index(self.player_stats.len());
        self.player_stats.insert(name, id);
        id
    }

    /// Returns `true` if `name` is a registered player stat.
    pub fn has_player_stat(&self, name: &str) -> bool {
        self.player_stats.contains_key(name)
    }

    /// Returns the [`PlayerStatId`] for the given player stat name, or `None` if
    /// not registered.
    pub fn player_stat(&self, name: &str) -> Option<PlayerStatId> {
        self.player_stats.get(name).copied()
    }

    /// Registers an entity buff definition by name and returns its assigned
    /// [`EntityBuffId`]. Ids are assigned in registration order, so identical
    /// content registered in the same order resolves to identical ids
    /// everywhere. Re-registering a name keeps the first definition and returns
    /// its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_entity_buff(
        &mut self,
        name: impl Into<String>,
        buff: EntityBuffDef,
    ) -> EntityBuffId {
        let name = name.into();
        assert!(!name.is_empty(), "entity buff name must not be empty");

        if let Some(&id) = self.entity_buffs.get(&name) {
            return id;
        }

        let id = EntityBuffId::from_index(self.entity_buff_defs.len());
        self.entity_buffs.insert(name, id);
        self.entity_buff_defs.push(buff);
        id
    }

    /// Returns `true` if `name` is a registered entity buff.
    pub fn has_entity_buff(&self, name: &str) -> bool {
        self.entity_buffs.contains_key(name)
    }

    /// Returns the [`EntityBuffId`] for the given entity buff name, or `None`
    /// if not registered.
    pub fn entity_buff(&self, name: &str) -> Option<EntityBuffId> {
        self.entity_buffs.get(name).copied()
    }

    /// Returns the entity buff definition for the given handle.
    pub fn entity_buff_def(&self, id: EntityBuffId) -> &EntityBuffDef {
        &self.entity_buff_defs[id.index()]
    }

    /// Returns the name the given entity buff is registered under, or `None`
    /// if the handle did not come from this registry.
    pub fn entity_buff_name(&self, id: EntityBuffId) -> Option<&str> {
        self.entity_buffs
            .iter()
            .find(|&(_, &buff)| buff == id)
            .map(|(name, _)| name.as_str())
    }

    /// Registers a player buff definition by name and returns its assigned
    /// [`PlayerBuffId`]. Ids are assigned in registration order, so identical
    /// content registered in the same order resolves to identical ids
    /// everywhere. Re-registering a name keeps the first definition and returns
    /// its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_player_buff(
        &mut self,
        name: impl Into<String>,
        buff: PlayerBuffDef,
    ) -> PlayerBuffId {
        let name = name.into();
        assert!(!name.is_empty(), "player buff name must not be empty");

        if let Some(&id) = self.player_buffs.get(&name) {
            return id;
        }

        let id = PlayerBuffId::from_index(self.player_buff_defs.len());
        self.player_buffs.insert(name, id);
        self.player_buff_defs.push(buff);
        id
    }

    /// Returns `true` if `name` is a registered player buff.
    pub fn has_player_buff(&self, name: &str) -> bool {
        self.player_buffs.contains_key(name)
    }

    /// Returns the [`PlayerBuffId`] for the given player buff name, or `None`
    /// if not registered.
    pub fn player_buff(&self, name: &str) -> Option<PlayerBuffId> {
        self.player_buffs.get(name).copied()
    }

    /// Returns the player buff definition for the given handle.
    pub fn player_buff_def(&self, id: PlayerBuffId) -> &PlayerBuffDef {
        &self.player_buff_defs[id.index()]
    }

    /// Registers a projectile definition by name and returns its assigned
    /// [`ProjectileId`]. Ids are assigned in registration order, so identical content
    /// registered in the same order resolves to identical ids everywhere.
    /// Re-registering a name keeps the first definition and returns its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_projectile(
        &mut self,
        name: impl Into<String>,
        projectile: ProjectileDef,
    ) -> ProjectileId {
        let name = name.into();
        assert!(!name.is_empty(), "projectile name must not be empty");

        if let Some(&id) = self.projectiles.get(&name) {
            return id;
        }

        let id = ProjectileId::from_index(self.projectile_defs.len());
        self.projectiles.insert(name, id);
        self.projectile_defs.push(projectile);
        id
    }

    /// Returns the [`ProjectileId`] for the given name, or `None` if not registered.
    pub fn projectile(&self, name: &str) -> Option<ProjectileId> {
        self.projectiles.get(name).copied()
    }

    /// Returns the name the given projectile is registered under, or `None` if the
    /// handle did not come from this registry.
    pub fn projectile_name(&self, id: ProjectileId) -> Option<&str> {
        self.projectiles
            .iter()
            .find(|&(_, &projectile)| projectile == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the projectile definition for the given handle.
    pub fn projectile_def(&self, id: ProjectileId) -> &ProjectileDef {
        &self.projectile_defs[id.index()]
    }

    /// Registers a turret definition by name and returns its assigned
    /// [`TurretId`]. Ids are assigned in registration order, so identical content
    /// registered in the same order resolves to identical ids everywhere.
    /// Re-registering a name keeps the first definition and returns its existing id.
    ///
    /// Panics if `name` is empty.
    pub fn register_turret(&mut self, name: impl Into<String>, turret: TurretDef) -> TurretId {
        let name = name.into();
        assert!(!name.is_empty(), "turret name must not be empty");

        if let Some(&id) = self.turrets.get(&name) {
            return id;
        }

        let id = TurretId::new(self.turret_defs.len());
        self.turrets.insert(name, id);
        self.turret_defs.push(turret);
        id
    }

    /// Returns the name the given entity stat is registered under, or `None` if
    /// the handle did not come from this registry.
    pub fn entity_stat_name(&self, id: EntityStatId) -> Option<&str> {
        self.entity_stats
            .iter()
            .find(|&(_, &stat)| stat == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the [`TurretId`] for the given name, or `None` if not registered.
    pub fn turret(&self, name: &str) -> Option<TurretId> {
        self.turrets.get(name).copied()
    }

    /// Returns the name the given turret is registered under, or `None` if the
    /// handle did not come from this registry.
    pub fn turret_name(&self, id: TurretId) -> Option<&str> {
        self.turrets
            .iter()
            .find(|&(_, &turret)| turret == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the turret definition for the given handle.
    pub fn turret_def(&self, id: TurretId) -> &TurretDef {
        &self.turret_defs[id.index()]
    }

    /// Every weapon `def` carries: the body's own, then each turret's in mounted
    /// order.
    pub fn weapons_of<'a>(&'a self, def: &'a EntityTypeDef) -> impl Iterator<Item = &'a Weapon> {
        def.attack.iter().map(AttackDef::weapon).chain(
            def.turrets
                .iter()
                .map(|mount| self.turret_def(mount.turret()).weapon()),
        )
    }

    /// Every layer the weapons `def` carries can reach between them — the body's
    /// own and every turret's.
    pub fn targets_of(&self, def: &EntityTypeDef) -> LayerMask {
        self.weapons_of(def)
            .fold(LayerMask::EMPTY, |reach, weapon| reach | weapon.targets())
    }

    /// Whether `weapon`'s shots are sent to a place rather than after a body —
    /// the only kind of weapon that can be aimed at bare ground.
    pub fn weapon_aims_at_cells(&self, weapon: &Weapon) -> bool {
        match weapon.delivery() {
            Delivery::Projectile(projectile) => {
                self.projectile_def(projectile).aim() == Aim::Position
            }
            Delivery::Instant => false,
        }
    }

    /// Registers a skill definition by name and returns its assigned
    /// [`SkillId`]. Ids are assigned in registration order, so identical
    /// content registered in the same order resolves to identical ids
    /// everywhere. Re-registering a name keeps the first definition and
    /// returns its existing id.
    ///
    /// Panics if `name` is empty, the skill costs an unregistered resource
    /// kind, or its effect references a buff this registry never minted.
    pub fn register_skill(&mut self, name: impl Into<String>, skill: SkillDef) -> SkillId {
        let name = name.into();
        assert!(!name.is_empty(), "skill name must not be empty");

        match &skill.caster {
            SkillCaster::Entity { costs, effect, .. } => {
                for cost in costs {
                    match cost {
                        EntityCastCost::Resources(resources) => {
                            for kind in resources.keys() {
                                assert!(
                                    self.has_resource(kind),
                                    "skill '{name}' costs unregistered resource kind '{kind}'"
                                );
                            }
                        }
                        // Whether the pool exists is the carrying type's
                        // business, checked when a type declares the skill
                        // (see [`Self::validate_skills`]).
                        EntityCastCost::Energy(_) | EntityCastCost::Health(_) => {}
                    }
                }
                match effect {
                    EntityCastEffect::ApplyBuff(buff) | EntityCastEffect::RemoveBuff(buff) => {
                        assert!(
                            buff.index() < self.entity_buff_defs.len(),
                            "skill '{name}' references an unregistered entity buff"
                        )
                    }
                    EntityCastEffect::Damage(_) | EntityCastEffect::Heal(_) => {}
                }
            }
            SkillCaster::Player { cost, effect } => {
                for kind in cost.keys() {
                    assert!(
                        self.has_resource(kind),
                        "skill '{name}' costs unregistered resource kind '{kind}'"
                    );
                }
                match effect {
                    PlayerCastEffect::ApplyBuff(buff) | PlayerCastEffect::RemoveBuff(buff) => {
                        assert!(
                            buff.index() < self.player_buff_defs.len(),
                            "skill '{name}' references an unregistered player buff"
                        )
                    }
                }
            }
        }

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

    /// Returns the [`SkillId`] for the given skill name, or `None` if not
    /// registered.
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

    /// Returns the skill definition for the given handle, or `None` if the
    /// handle did not come from this registry.
    pub fn skill_def(&self, id: SkillId) -> Option<&SkillDef> {
        self.skill_defs.get(id.index())
    }

    /// Returns every registered skill with its handle, in ascending name order.
    pub fn skills(&self) -> impl Iterator<Item = (&str, SkillId)> {
        self.skills.iter().map(|(name, &id)| (name.as_str(), id))
    }

    /// Registers a research definition by name and returns its assigned
    /// [`ResearchId`]. Ids are assigned in registration order, so identical
    /// content registered in the same order resolves to identical ids
    /// everywhere. Re-registering a name keeps the first definition and
    /// returns its existing id.
    ///
    /// The research's requirements are forward references, validated by
    /// [`validate`](Self::validate) once all content is registered.
    ///
    /// Panics if `name` is empty, the research costs an unregistered resource
    /// kind, or it applies a buff this registry never minted.
    pub fn register_research(
        &mut self,
        name: impl Into<String>,
        research: ResearchDef,
    ) -> ResearchId {
        let name = name.into();
        assert!(!name.is_empty(), "research name must not be empty");

        for kind in research.cost.keys() {
            assert!(
                self.has_resource(kind),
                "research '{name}' costs unregistered resource kind '{kind}'"
            );
        }
        if let Some(buff) = research.buff {
            assert!(
                buff.index() < self.player_buff_defs.len(),
                "research '{name}' references an unregistered player buff"
            );
        }

        if let Some(&id) = self.researches.get(&name) {
            return id;
        }

        let id = ResearchId::from_index(self.research_defs.len());
        self.researches.insert(name, id);
        self.research_defs.push(research);
        id
    }

    /// Returns `true` if `name` is a registered research.
    pub fn has_research(&self, name: &str) -> bool {
        self.researches.contains_key(name)
    }

    /// Returns the [`ResearchId`] for the given research name, or `None` if
    /// not registered.
    pub fn research(&self, name: &str) -> Option<ResearchId> {
        self.researches.get(name).copied()
    }

    /// Returns the research definition for the given handle, or `None` if the
    /// handle did not come from this registry.
    pub fn research_def(&self, id: ResearchId) -> Option<&ResearchDef> {
        self.research_defs.get(id.index())
    }

    /// Returns the name the given research is registered under, or `None` if
    /// the handle did not come from this registry.
    pub fn research_name(&self, id: ResearchId) -> Option<&str> {
        self.researches
            .iter()
            .find(|&(_, &research)| research == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns every registered research with its handle, in ascending name
    /// order.
    pub fn researches(&self) -> impl Iterator<Item = (&str, ResearchId)> {
        self.researches
            .iter()
            .map(|(name, &id)| (name.as_str(), id))
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
        // A mover's footprint must be square. Clearance is one number per
        // mover, its body is a circle inscribed in the footprint, and the crowd
        // ladder compares footprints as interchangeable — all of which hold for
        // a square and none of which hold for an oblong, which would additionally
        // need a rule for whether the footprint turns with the mover.
        assert!(
            !def.can_move()
                || def
                    .location
                    .is_some_and(|l| l.size().width == l.size().height),
            "entity type '{}' moves but has a non-square footprint",
            def.name
        );
    }

    /// Checks each transition a type offers: the destination is registered and
    /// reachable across the footprint change, the requirements resolve, and the
    /// pools the costs draw from exist.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because
    /// transitions may be circular: two forms can each name the other, so
    /// neither could be registered first.
    fn validate_morphs(&self, def: &EntityTypeDef) {
        for morph in &def.morphs {
            let owner = format!(
                "entity type '{}' morphing into '{}'",
                def.name,
                morph.into_type()
            );
            let other = self
                .entity(morph.into_type())
                .unwrap_or_else(|| panic!("{owner} names a type that is not registered"));
            // The changed form is recentred so its middle stays put, which shifts
            // the anchor by half the size difference per axis. Only an even
            // difference keeps that shift in whole cells; an odd one strands the
            // anchor between lattice points, where the cell model has no footprint.
            if let (Some(from), Some(to)) = (def.location, other.location) {
                let even_shift = |a: u32, b: u32| a.abs_diff(b).is_multiple_of(2);
                assert!(
                    even_shift(from.size().width, to.size().width)
                        && even_shift(from.size().height, to.size().height),
                    "{owner} crosses an odd footprint difference, which has no \
                     whole-cell anchor to land on"
                );
            }
            self.validate_requires(&owner, morph.requires());
            // A time read from a stat the type never declares would silently
            // mean an instant change — the same validates-but-lies class as a
            // cost without its pool.
            if let MorphTime::Stat(stat) = morph.time() {
                assert!(
                    def.base_stats.contains_key(&stat),
                    "{owner} reads its time from a stat the type does not carry"
                );
            }
            for cost in morph.costs() {
                match cost {
                    EntityCastCost::Resources(resources) => {
                        for kind in resources.keys() {
                            assert!(
                                self.has_resource(kind),
                                "{owner} costs unregistered resource kind '{kind}'"
                            );
                        }
                    }
                    EntityCastCost::Energy(_) => assert!(
                        def.has_energy(),
                        "{owner} has an energy cost but no max_energy stat"
                    ),
                    EntityCastCost::Health(_) => assert!(
                        def.has_health(),
                        "{owner} has a health cost but no health pool"
                    ),
                }
            }
        }
    }

    /// Checks that a mover could stand somewhere: some registered terrain has to
    /// pass every layer it occupies.
    ///
    /// An occupation mask is conjunctive — a mover needs all of its layers free —
    /// so a combined mask names terrain that passes *all* of them, not terrain
    /// that passes any. A ground-and-water mover is therefore a shore unit, and
    /// wants a shore terrain to exist; without one it could not stand anywhere on
    /// any map, which is a content mistake rather than a situation to discover at
    /// runtime as a unit that mysteriously never moves.
    ///
    /// Content that declares no terrain at all is exempt: a map without terrain
    /// starts fully open, so every layer is passable everywhere and there is
    /// nothing to be inconsistent with.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because
    /// terrains and entity types may be declared in either order.
    fn validate_traversable(&self, def: &EntityTypeDef) {
        if !def.can_move() || self.terrains.is_empty() {
            return;
        }
        let Some(occupation) = def.location.map(|location| location.occupation()) else {
            return;
        };
        assert!(
            self.terrains
                .values()
                .any(|&passable| passable & occupation == occupation),
            "entity type '{}' moves on layers {occupation} that no registered terrain \
             passes together, so it could never stand anywhere",
            def.name
        );
    }

    /// Checks that a type's delivery configuration is usable: a blast must reach
    /// registered layers.
    ///
    /// That only an attacker delivers anything at all needs no check: the
    /// delivery and the blast are parts of the weapon, so a type without one
    /// cannot state either.
    fn validate_delivery(&self, def: &EntityTypeDef) {
        for weapon in self.weapons_of(def) {
            if let Some(splash) = weapon.splash() {
                let unregistered = splash.layers() & !self.registered_layers();
                assert!(
                    unregistered == LayerMask::EMPTY,
                    "entity type '{}' splashes onto unregistered layers {unregistered}",
                    def.name
                );
            }
        }

        for (mask, what) in [
            ((def.can_attack()).then(|| self.targets_of(def)), "targets"),
            (def.targetable, "is targetable on"),
        ] {
            let Some(mask) = mask else { continue };
            let unregistered = mask & !self.registered_layers();
            assert!(
                unregistered == LayerMask::EMPTY,
                "entity type '{}' {what} unregistered layers {unregistered}",
                def.name
            );
        }
    }

    /// Checks that every skill the type carries is entity-cast and can be paid
    /// for: each pool cost draws from the caster, so the type must have the
    /// pools its skills spend from.
    fn validate_skills(&self, def: &EntityTypeDef) {
        for &skill in &def.skills {
            let skill_def = self
                .skill_def(skill)
                .expect("a declared skill id must come from this registry");
            let costs = match &skill_def.caster {
                SkillCaster::Entity { costs, .. } => costs,
                SkillCaster::Player { .. } => panic!(
                    "entity type '{}' declares player-cast skill '{}'",
                    def.name,
                    self.skill_name(skill).unwrap_or("<unregistered>"),
                ),
            };
            for cost in costs {
                match cost {
                    // Kinds were checked when the skill was registered; the
                    // stockpile is the owner's, not the type's, so there is
                    // nothing type-level left to require.
                    EntityCastCost::Resources(_) => {}
                    EntityCastCost::Energy(_) => assert!(
                        def.has_energy(),
                        "entity type '{}' has skill '{}' with an energy cost but no max_energy stat",
                        def.name,
                        self.skill_name(skill).unwrap_or("<unregistered>"),
                    ),
                    EntityCastCost::Health(_) => assert!(
                        def.has_health(),
                        "entity type '{}' has skill '{}' with a health cost but no health pool",
                        def.name,
                        self.skill_name(skill).unwrap_or("<unregistered>"),
                    ),
                }
            }
        }
    }

    /// Checks that every research the type can host was minted by this registry.
    fn validate_researcher(&self, def: &EntityTypeDef) {
        let Some(researcher) = &def.researcher else {
            return;
        };

        for research in researcher.researches() {
            assert!(
                research.index() < self.research_defs.len(),
                "entity type '{}' hosts an unregistered research",
                def.name
            );
        }
    }

    /// Checks that every requirement entry resolves to exactly one vocabulary:
    /// a research, or an entity type or tag (the two entity readings share
    /// their meaning, so a name serving both is fine; a name that is both a
    /// research and an entity term would leave content ambiguous).
    fn validate_requires(&self, owner: &str, requires: &[String]) {
        for name in requires {
            let research = self.researches.contains_key(name);
            let entity_term = self.defs_by_name.contains_key(name) || self.tags.contains(name);
            assert!(
                research || entity_term,
                "{owner} requires '{name}', which is not a registered entity type, tag, \
                 or research"
            );
            assert!(
                !(research && entity_term),
                "{owner} requires '{name}', which names both a research and an entity \
                 type or tag"
            );
        }
    }

    /// Checks that every bonus a type names can ever be matched: a key stands for
    /// a registered entity type or a registered tag, since a hit is judged against
    /// its victim's type name and its tags and nothing else.
    ///
    /// A name meaning both is fine — the bonus applies once either way — which is
    /// why this is looser than [`validate_requires`](Self::validate_requires),
    /// where naming two things leaves the requirement genuinely unclear.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because a
    /// bonus may name a type registered after the attacker that fears it.
    fn validate_bonus_damage_vs(&self, def: &EntityTypeDef) {
        for name in def.bonus_damage_vs.keys() {
            assert!(
                self.defs_by_name.contains_key(name) || self.tags.contains(name),
                "entity type '{}' deals bonus damage to '{name}', which is not a \
                 registered entity type or tag, so the bonus could never apply",
                def.name
            );
        }
    }

    /// Checks the engine's built-in stats: a declared pool or speed is positive (a
    /// zero would be meaningless); a stat the engine reads as a whole number is at
    /// least its floor; an attacker — one carrying the [`EntityStatId::DAMAGE`] stat —
    /// also carries the rest of its weapon; and the hit lands within the attack
    /// cycle (`damage_point <= attack_period`).
    /// Content's own custom stats are engine-transparent and not checked here.
    fn validate_stats(&self, def: &EntityTypeDef) {
        // Declaring any of these at zero says nothing an omitted stat would not
        // — or, for a capacity, declares a capability that can never act.
        for stat in [
            EntityStatId::MAX_HEALTH,
            EntityStatId::SPEED,
            EntityStatId::MAX_ENERGY,
            EntityStatId::REPAIR_SPEED,
            EntityStatId::SUPPLY_PROVIDED,
            EntityStatId::SUPPLY_COST,
            EntityStatId::CARGO_CAPACITY,
        ] {
            if let Some(value) = def.base_stat(stat) {
                assert!(
                    value > FixedU64::ZERO,
                    "entity type '{}' has a non-positive {} stat",
                    def.name,
                    ENTITY_BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        // A floored stat is one the engine reads as a whole number, so an authored
        // value below the floor truncates to something its consumer can never
        // satisfy — and an entity that is never buffed never reaches the fold that
        // would raise it. Driven off the floor table so the two cannot disagree.
        for builtin in &ENTITY_BUILTIN_STATS {
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

        // A limit nothing reads is a limit its author believes in. Each of these is
        // read by exactly one rule, so declaring it without the thing that rule
        // governs is an author expecting behaviour that can never happen — a gun
        // they think slews, a body they think lines up first.
        let bears_on_its_own = !def.turrets.is_empty();
        for (stat, governs, missing) in [
            (EntityStatId::DAMAGE, def.can_attack(), "has no weapon"),
            (EntityStatId::TURN_RATE, def.can_move(), "cannot move"),
            (EntityStatId::PIVOT_ANGLE, def.can_move(), "cannot move"),
            (EntityStatId::ATTACK_ARC, def.can_attack(), "has no weapon"),
            (
                EntityStatId::AIM_RATE,
                bears_on_its_own,
                "carries no turret",
            ),
        ] {
            assert!(
                governs || def.base_stat(stat).is_none(),
                "entity type '{}' declares {} but {missing}",
                def.name,
                ENTITY_BUILTIN_STATS[stat.index()].name,
            );
        }

        // A gun sits on the body that carries it, so it has to fit on it — and the
        // stats it reads have to be there to read.
        assert!(
            def.turrets.is_empty() || def.location.is_some(),
            "entity type '{}' mounts turrets but has no footprint to mount them on",
            def.name,
        );
        for mount in &def.turrets {
            let turret = self.turret_def(mount.turret());
            let footprint = def
                .location
                .expect("a type mounting turrets has a footprint")
                .size();
            let fits = mount.origin().x + mount.size().width <= footprint.width
                && mount.origin().y + mount.size().height <= footprint.height;
            assert!(
                fits,
                "entity type '{}' mounts a turret outside its own footprint",
                def.name,
            );
            // A turret that shoots while the body goes about its orders needs a
            // body that goes somewhere: on anything else the conduct is a
            // behaviour its author expects and nothing can honour.
            if matches!(turret.conduct(), WeaponConduct::OnTheMove) {
                assert!(
                    def.can_move(),
                    "entity type '{}' carries a turret that fires on the move but cannot move",
                    def.name,
                );
            }
            for (stat, what) in [
                (turret.stats().damage, "damage"),
                (turret.stats().range, "range"),
                (turret.stats().acquire_range, "acquisition range"),
                (turret.stats().period, "cycle"),
                (turret.stats().damage_point, "damage point"),
            ] {
                assert!(
                    def.base_stat(stat).is_some(),
                    "entity type '{}' carries a turret whose {what} reads {}, which it does not declare",
                    def.name,
                    self.entity_stat_name(stat)
                        .unwrap_or("a stat it never named"),
                );
            }
        }

        if def.can_move() {
            // Both rates are read every tick a body walks — one while it is
            // moving, one while it is standing — and omitting either would leave
            // the movement rules to guess separately at what the body can do.
            for stat in [EntityStatId::TURN_RATE, EntityStatId::PIVOT_RATE] {
                assert!(
                    def.base_stat(stat).is_some(),
                    "entity type '{}' carries the speed stat but is missing {}",
                    def.name,
                    ENTITY_BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        // A body's own weapon fights by the standard numbers, so a type that
        // points one declares them all. What a turret reads is checked against
        // the stats that turret names, beside the mount that carries it.
        //
        // What a weapon reaches is required and never defaulted, which the type
        // system now says for us: a weapon cannot be stated without its targets.
        if def.attack.is_some() {
            for stat in [
                EntityStatId::DAMAGE,
                EntityStatId::ATTACK_RANGE,
                EntityStatId::ACQUIRE_RANGE,
                EntityStatId::ATTACK_PERIOD,
                EntityStatId::DAMAGE_POINT,
            ] {
                assert!(
                    def.base_stat(stat).is_some(),
                    "entity type '{}' points a weapon but is missing {}",
                    def.name,
                    ENTITY_BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        // A regeneration rate is read through the pool it refills, so one declared
        // without that pool is content that can never take effect.
        for (regen, pool) in [
            (EntityStatId::HEALTH_REGEN, EntityStatId::MAX_HEALTH),
            (EntityStatId::ENERGY_REGEN, EntityStatId::MAX_ENERGY),
        ] {
            if def.base_stat(regen).is_some() {
                assert!(
                    def.base_stat(pool).is_some(),
                    "entity type '{}' declares {} without {}",
                    def.name,
                    ENTITY_BUILTIN_STATS[regen.index()].name,
                    ENTITY_BUILTIN_STATS[pool.index()].name,
                );
            }
        }

        if let (Some(period), Some(damage_point)) = (
            def.base_stat(EntityStatId::ATTACK_PERIOD),
            def.base_stat(EntityStatId::DAMAGE_POINT),
        ) {
            assert!(
                damage_point <= period,
                "entity type '{}' has a damage_point beyond its attack_period",
                def.name
            );
        }
    }

    /// Checks that a build capability carries the reach the order reads, and that the
    /// stat is not declared by something that cannot build.
    fn validate_build(&self, def: &EntityTypeDef) {
        if def.base_stat(EntityStatId::BUILD_RANGE).is_some() {
            assert!(
                def.builder.is_some(),
                "entity type '{}' declares build_range but cannot build",
                def.name
            );
        }
        if def.builder.is_some() {
            assert!(
                def.base_stat(EntityStatId::BUILD_RANGE).is_some(),
                "entity type '{}' can build but is missing build_range",
                def.name
            );
        }
    }

    /// Checks that a transport capability carries the stats the orders read, that
    /// none of those stats is declared by something that cannot transport, and
    /// that a transporter is not itself transportable.
    fn validate_transport(&self, def: &EntityTypeDef) {
        // Every transport stat is read only through a transport capability, so any
        // one alone is content that can never take effect.
        for stat in [
            EntityStatId::CARGO_CAPACITY,
            EntityStatId::LOAD_RANGE,
            EntityStatId::UNLOAD_RANGE,
            EntityStatId::LOAD_PERIOD,
            EntityStatId::UNLOAD_PERIOD,
        ] {
            if def.base_stat(stat).is_some() {
                assert!(
                    def.can_transport(),
                    "entity type '{}' declares {} but cannot transport",
                    def.name,
                    ENTITY_BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        if !def.can_transport() {
            return;
        }

        // All of them, the periods included: zero is the unmetered pace, and the
        // author writes it rather than having the engine assume it.
        for stat in [
            EntityStatId::CARGO_CAPACITY,
            EntityStatId::LOAD_RANGE,
            EntityStatId::UNLOAD_RANGE,
            EntityStatId::LOAD_PERIOD,
            EntityStatId::UNLOAD_PERIOD,
        ] {
            assert!(
                def.base_stat(stat).is_some(),
                "entity type '{}' can transport but is missing {}",
                def.name,
                ENTITY_BUILTIN_STATS[stat.index()].name,
            );
        }
        // A transporter riding inside another would nest holders; keeping the two
        // capabilities apart makes that unrepresentable.
        assert!(
            def.base_stat(EntityStatId::CARGO_SIZE).is_none(),
            "entity type '{}' can transport and so cannot declare cargo_size",
            def.name
        );
    }

    /// Checks that a carrying capability carries the reach the order reads, and that
    /// the stat is not declared by something that cannot carry.
    fn validate_harvest(&self, def: &EntityTypeDef) {
        if def.base_stat(EntityStatId::HARVEST_RANGE).is_some() {
            assert!(
                def.resource_carrier.is_some(),
                "entity type '{}' declares harvest_range but cannot carry resources",
                def.name
            );
        }
        if def.resource_carrier.is_some() {
            assert!(
                def.base_stat(EntityStatId::HARVEST_RANGE).is_some(),
                "entity type '{}' can carry resources but is missing harvest_range",
                def.name
            );
        }
    }

    /// Checks that a repair capability is complete and that the terms it names —
    /// mended tags, charged resources, and the target's own repair scale — resolve.
    fn validate_repair(&self, def: &EntityTypeDef) {
        // Both repair stats are read only through a repair capability, so either one
        // alone is content that can never take effect.
        for stat in [
            EntityStatId::REPAIR_SPEED,
            EntityStatId::REPAIR_COST_FACTOR,
            EntityStatId::REPAIR_RANGE,
        ] {
            if def.base_stat(stat).is_some() {
                assert!(
                    def.can_repair(),
                    "entity type '{}' declares {} but cannot repair",
                    def.name,
                    ENTITY_BUILTIN_STATS[stat.index()].name,
                );
            }
        }

        if let Some(ratio) = def.repair_ratio {
            assert!(
                ratio > FixedU64::ZERO,
                "entity type '{}' has a non-positive repair_ratio",
                def.name
            );
            assert!(
                def.production_time().is_some(),
                "entity type '{}' has a repair_ratio but no build_time or train_time \
                 to scale it against",
                def.name
            );
        }

        let Some(repairer) = def.repairer.as_ref() else {
            return;
        };

        for stat in [EntityStatId::REPAIR_SPEED, EntityStatId::REPAIR_RANGE] {
            assert!(
                def.base_stat(stat).is_some(),
                "entity type '{}' can repair but is missing {}",
                def.name,
                ENTITY_BUILTIN_STATS[stat.index()].name,
            );
        }
        for tag in repairer.repairs() {
            assert!(
                self.has_tag(tag),
                "entity type '{}' repairs unregistered tag '{tag}'",
                def.name
            );
        }
        match repairer.cost() {
            RepairCost::Free => {}
            // The factor is what turns a target's price into a repair bill, and it is
            // a stat so that an upgrade can move it.
            RepairCost::ProRata => assert!(
                def.base_stat(EntityStatId::REPAIR_COST_FACTOR).is_some(),
                "entity type '{}' charges pro-rata repair but is missing \
                 repair_cost_factor",
                def.name
            ),
            RepairCost::PerTick(cost) => {
                for kind in cost.keys() {
                    assert!(
                        self.has_resource(kind),
                        "entity type '{}' charges unregistered resource kind '{kind}' \
                         for repair",
                        def.name
                    );
                }
            }
            // Spending from a pool the type does not have would make the work free.
            RepairCost::Energy(_) => assert!(
                def.has_energy(),
                "entity type '{}' pays for repair with energy but has no max_energy \
                 stat",
                def.name
            ),
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

    /// Checks that every admission-list entry resolves to a registered entity
    /// type or tag. Carried types may register after their carrier, so this
    /// runs in the deferred pass.
    fn validate_carries(&self, def: &EntityTypeDef) {
        let Some(transporter) = &def.transporter else {
            return;
        };

        for name in transporter.carries() {
            assert!(
                self.defs_by_name.contains_key(name) || self.tags.contains(name),
                "entity type '{}' carries '{name}', which is not a registered entity type or tag",
                def.name
            );
        }
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
