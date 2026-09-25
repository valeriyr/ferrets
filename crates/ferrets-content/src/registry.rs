//! Registry of all content-defined entity types and resource kinds.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};

use crate::{
    annex::{AloneConduct, AnnexLife},
    attack::{AttackDef, Delivery, Weapon},
    brood::BroodlingDef,
    build::BuilderAttendance,
    cost::Cost,
    detection::Detection,
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_stats::{ENTITY_BUILTIN_STATS, EntityStatDef, EntityStatId},
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::{Emission, FieldDef, FieldGrowth, FieldId, FieldLayer},
    kinds::{Kind, Kinds},
    morph::MorphPlacement,
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    player_stats::{PLAYER_BUILTIN_STATS, PlayerStatId},
    projectile::{Aim, ProjectileDef, ProjectileId},
    quantity::Quantity,
    repair::RepairCost,
    requirement::{Requirement, Scope},
    research::{ResearchDef, ResearchId},
    skills::{
        Casting, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach, SkillCaster,
        SkillDef, SkillId,
    },
    stand::StandingAct,
    tags,
    turret::{TurretDef, TurretId, WeaponConduct},
    work::{Crewing, WorkPresence},
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
    fields: BTreeMap<String, FieldId>,
    field_defs: Vec<FieldDef>,
    entity_stats: BTreeMap<String, EntityStatId>,
    /// What each entity stat is, by registration index — the builtins first,
    /// then what content declares.
    entity_stat_defs: Vec<EntityStatDef>,
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
            tags: BTreeSet::from([tags::BUILDING.to_string(), tags::REMAINS.to_string()]),
            layers: BTreeMap::new(),
            terrains: BTreeMap::new(),
            fields: BTreeMap::new(),
            field_defs: Vec::new(),
            entity_stats: ENTITY_BUILTIN_STATS
                .iter()
                .map(|builtin| (builtin.name.to_string(), builtin.id))
                .collect(),
            entity_stat_defs: ENTITY_BUILTIN_STATS
                .iter()
                .map(|builtin| EntityStatDef::new(builtin.floor))
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
    /// acyclic hierarchy — so resource kinds it references must be registered
    /// first. What a death leaves is *not* checked here, because a type may be
    /// registered before what it hands on; [`validate`] checks that, and that
    /// the decay chains it starts bottom out.
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
    /// layers, is remains without a lifetime, with live-gameplay data on it or
    /// with a dying time of its own, or projects, reads, or answers to a field
    /// this registry never minted.
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
        self.validate_remains(&def);
        self.validate_stats(&def);
        self.validate_skills(&def);
        self.validate_researcher(&def);
        self.validate_delivery(&def);
        self.validate_repair(&def);
        self.validate_build(&def);
        self.validate_harvest(&def);
        self.validate_transport(&def);
        self.validate_fields(&def);

        let id = EntityTypeId::from_index(self.defs.len());
        self.defs_by_name.insert(def.name.clone(), id);
        self.defs.push(def);
    }

    /// Validates every name a registered type, research or skill points at,
    /// and every declaration a type makes about itself. Call once after
    /// everything has been registered.
    ///
    /// These references cannot be checked at registration time: production
    /// catalogues may form cycles, and a filter may name a type registered
    /// after it — or the very type that carries it. This pass checks them
    /// against the complete registry, in any registration order.
    ///
    /// Panics, naming the offending type and reference, when any name a
    /// registered type points at does not resolve against the complete
    /// registry, or when a declaration cannot be carried out by the type that
    /// makes it. Each `validate_*` below states the terms it enforces.
    pub fn validate(&self) {
        for def in &self.defs {
            self.validate_trains(def);
            self.validate_builds(def);
            self.validate_carries(def);
            self.validate_repairs(def);
            self.validate_requires(&format!("entity type '{}'", def.name), &def.requires);
            self.validate_bonus_damage_vs(def);
            self.validate_traversable(def);
            self.validate_morphs(def);
            self.validate_crews(def);
            self.validate_berths(def);
            self.validate_harvest_sources(def);
            self.validate_overbuilds(def);
            self.validate_docks(def);
            self.validate_annex(def);
            self.validate_breeder(def);
            self.validate_broodling(def);
            self.validate_leaves(def);
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
            self.validate_aim(name, &self.skill_defs[id.index()]);
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
    /// content-declared stats follow.
    ///
    /// `floor` is the smallest effective value it may fold to: a non-zero one
    /// marks a stat whose zero the consumer can never mean — a reach of
    /// nothing, a cast worked over no time — so a debuff deep enough to reach
    /// it holds there instead.
    ///
    /// Panics if `name` is empty, is already registered — a second declaration
    /// would leave the floor depending on which of them ran first — or already
    /// names a player stat, the two vocabularies being separate.
    pub fn register_entity_stat(
        &mut self,
        name: impl Into<String>,
        floor: FixedU64,
    ) -> EntityStatId {
        let name = name.into();
        assert!(!name.is_empty(), "stat name must not be empty");
        assert!(
            !self.player_stats.contains_key(&name),
            "'{name}' is already registered as a player stat"
        );
        assert!(
            !self.entity_stats.contains_key(&name),
            "entity stat '{name}' is already registered"
        );

        let id = EntityStatId::from_index(self.entity_stats.len());
        self.entity_stats.insert(name, id);
        self.entity_stat_defs.push(EntityStatDef::new(floor));
        id
    }

    /// What every registered entity stat is, by registration index.
    pub fn entity_stat_defs(&self) -> &[EntityStatDef] {
        &self.entity_stat_defs
    }

    /// What `stat` is, as it was registered.
    pub fn entity_stat_def(&self, stat: EntityStatId) -> EntityStatDef {
        self.entity_stat_defs
            .get(stat.index())
            .copied()
            .expect("a stat id comes from this registry")
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
    /// Panics if `name` is empty, the buff lasts for no tick at all, its
    /// upkeep names nothing or an unregistered resource kind, a modifier list
    /// is empty or touches an unregistered entity stat, or the buff does
    /// nothing at all — no effect, no upkeep, and no interruption.
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

        match &buff.lasting {
            Lasting::For(ticks) => {
                assert!(*ticks > 0, "entity buff '{name}' lasts for no time at all")
            }
            Lasting::Upkeep { costs, period } => {
                assert!(
                    !costs.is_empty(),
                    "entity buff '{name}' has an upkeep that costs nothing"
                );
                assert!(
                    *period > 0,
                    "entity buff '{name}' pays its upkeep every zero ticks"
                );
                for cost in costs {
                    match cost {
                        Cost::Resources(resources) => {
                            for kind in resources.keys() {
                                assert!(
                                    self.has_resource(kind),
                                    "entity buff '{name}' costs unregistered resource kind '{kind}'"
                                );
                            }
                        }
                        // Whether the pool exists is the carrying type's
                        // business, checked when a type declares a skill that
                        // applies the buff (see [`Self::validate_skills`]).
                        Cost::Energy(_) | Cost::Health(_) => {}
                    }
                }
            }
            Lasting::Forever => {}
        }
        for effect in &buff.effects {
            match effect {
                EntityEffect::Modifiers(modifiers) => {
                    assert!(
                        !modifiers.is_empty(),
                        "entity buff '{name}' has an effect with no modifiers"
                    );
                    for modifier in modifiers {
                        assert!(
                            modifier.stat.index() < self.entity_stats.len(),
                            "entity buff '{name}' modifies an unregistered entity stat"
                        );
                    }
                }
                EntityEffect::Disable | EntityEffect::Conceal => {}
            }
        }
        // A buff that does nothing and ends on nothing is a name and no more;
        // one with an upkeep at least drains, and one with an interruption at
        // least marks that it was cut short.
        assert!(
            !buff.effects.is_empty()
                || matches!(buff.lasting, Lasting::Upkeep { .. })
                || !buff.interrupted_by.is_empty(),
            "entity buff '{name}' does nothing at all"
        );

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
    /// kind, its effect references a buff this registry never minted, or its
    /// watch detects on an unregistered layer.
    pub fn register_skill(&mut self, name: impl Into<String>, skill: SkillDef) -> SkillId {
        let name = name.into();
        assert!(!name.is_empty(), "skill name must not be empty");

        match &skill.caster {
            SkillCaster::Entity {
                costs,
                target,
                reach: _,
                casting,
                effect,
            } => {
                // A cast that lands on no tick at all never lands: a number
                // content spells out is held to that here, and a stat is held
                // to it by the floor it was registered with.
                if let Casting::Delayed { point, period } = casting {
                    // The least either ever folds to: a number content spells
                    // out is that number, and a stat is the floor it was
                    // registered with.
                    let least = |quantity: &Quantity| match quantity {
                        Quantity::Constant(value) => FixedU64::from_num(*value),
                        Quantity::Stat(stat) => self.entity_stat_def(*stat).floor(),
                    };
                    assert!(
                        least(point) >= FixedU64::ONE,
                        "skill '{name}' is cast over no time at all: an instant cast declares no `cast`, and a point read from a stat is held to that stat's floor"
                    );
                    assert!(
                        least(period) >= least(point),
                        "skill '{name}' frees its caster before the cast lands: {} < {}",
                        least(period),
                        least(point),
                    );
                }
                for cost in costs {
                    match cost {
                        Cost::Resources(resources) => {
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
                        Cost::Energy(_) | Cost::Health(_) => {}
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
                    EntityCastEffect::Field { field, .. } => assert!(
                        field.index() < self.field_defs.len(),
                        "skill '{name}' acts on an unregistered field"
                    ),
                    EntityCastEffect::Watch {
                        duration,
                        detection,
                        ..
                    } => {
                        assert!(*duration > 0, "skill '{name}' watches for no time at all");
                        self.validate_detection(&format!("skill '{name}'"), *detection);
                    }
                    EntityCastEffect::Summon { entity_type, count } => {
                        let summoned = self.defs.get(entity_type.index()).unwrap_or_else(|| {
                            panic!("skill '{name}' summons an unregistered entity type")
                        });
                        assert!(
                            summoned.location.is_some(),
                            "skill '{name}' summons '{}', which stands nowhere",
                            summoned.name
                        );
                        assert!(
                            !summoned.is_remains(),
                            "skill '{name}' summons '{}', which is remains: a cast raises something from a body, it does not lay one",
                            summoned.name
                        );
                        assert!(*count > 0, "skill '{name}' summons nothing at all");
                    }
                }
                // A cell has no pools to buff, damage, or heal; only a field
                // action and a watch land on one.
                match (target, effect) {
                    (
                        EntityCastTarget::Position,
                        EntityCastEffect::Field { .. }
                        | EntityCastEffect::Watch { .. }
                        | EntityCastEffect::Summon { .. },
                    ) => {}
                    (
                        EntityCastTarget::Position,
                        EntityCastEffect::ApplyBuff(_)
                        | EntityCastEffect::RemoveBuff(_)
                        | EntityCastEffect::Damage(_)
                        | EntityCastEffect::Heal(_),
                    ) => panic!("skill '{name}' aims at a position but its effect needs an entity"),
                    // What lies where it fell has no pools and no ground of its
                    // own to act on: what a cast does with a body is raise
                    // something from it.
                    (EntityCastTarget::Fallen { .. }, EntityCastEffect::Summon { .. }) => {}
                    (
                        EntityCastTarget::Fallen { .. },
                        EntityCastEffect::ApplyBuff(_)
                        | EntityCastEffect::RemoveBuff(_)
                        | EntityCastEffect::Damage(_)
                        | EntityCastEffect::Heal(_)
                        | EntityCastEffect::Field { .. }
                        | EntityCastEffect::Watch { .. },
                    ) => panic!("skill '{name}' aims at the fallen, which only a summon may spend"),
                    (EntityCastTarget::Caster | EntityCastTarget::Standing { .. }, _) => {}
                }
            }
            SkillCaster::Player { price, effect } => {
                for kind in price.keys() {
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
                // A player cast has no acting entity, so there is nothing for
                // an actor-scoped requirement to be asked of.
                assert!(
                    skill
                        .requires
                        .iter()
                        .all(|entry| matches!(entry.scope(), Scope::Player)),
                    "player-cast skill '{name}' requires something of an acting entity"
                );
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

        for kind in research.price.keys() {
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

    /// Registers a field kind (creep, power, …) by name and returns its
    /// assigned [`FieldId`]. Ids are assigned in registration order, so
    /// identical content registered in the same order resolves to identical
    /// ids everywhere.
    ///
    /// The layers the field covers must be registered first.
    ///
    /// Panics if `name` is empty, the field is already registered, it lies on
    /// an unregistered layer, or it detects on one.
    pub fn register_field(&mut self, name: impl Into<String>, field: FieldDef) -> FieldId {
        let name = name.into();
        assert!(!name.is_empty(), "field name must not be empty");
        assert!(
            !self.fields.contains_key(&name),
            "field '{name}' is already registered"
        );
        match field.layer() {
            FieldLayer::Anywhere => {}
            FieldLayer::Passable(layers) => {
                let unregistered = layers & !self.registered_layers();
                assert!(
                    unregistered == LayerMask::EMPTY,
                    "field '{name}' covers unregistered layers {unregistered}"
                );
            }
        }
        self.validate_detection(&format!("field '{name}'"), field.detection());

        let id = FieldId::from_index(self.field_defs.len());
        self.fields.insert(name, id);
        self.field_defs.push(field);
        id
    }

    /// Returns the [`FieldId`] for the given field name, or `None` if not
    /// registered.
    pub fn field(&self, name: &str) -> Option<FieldId> {
        self.fields.get(name).copied()
    }

    /// Returns the name the given field is registered under, or `None` if the
    /// handle did not come from this registry.
    pub fn field_name(&self, id: FieldId) -> Option<&str> {
        self.fields
            .iter()
            .find(|&(_, &field)| field == id)
            .map(|(name, _)| name.as_str())
    }

    /// Returns the field definition for the given handle.
    ///
    /// Panics if the handle did not come from this registry.
    pub fn field_def(&self, id: FieldId) -> &FieldDef {
        &self.field_defs[id.index()]
    }

    /// Every registered field handle, in registration order.
    pub fn field_ids(&self) -> impl Iterator<Item = FieldId> {
        (0..self.field_defs.len()).map(FieldId::from_index)
    }

    /// Every field whose detection reveals something, with the layers it
    /// reveals, in registration order.
    pub fn detecting_fields(&self) -> impl Iterator<Item = (FieldId, LayerMask)> + '_ {
        self.field_defs
            .iter()
            .enumerate()
            .filter_map(|(index, field)| match field.detection() {
                Detection::Blind => None,
                Detection::Reveals(layers) => Some((FieldId::from_index(index), layers)),
            })
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
            let into = self
                .entity(morph.into_type())
                .unwrap_or_else(|| panic!("{owner} names a type that is not registered"));
            assert!(
                !into.is_remains(),
                "{owner} names remains, which only a death may leave"
            );
            match morph.placement() {
                MorphPlacement::Nearby => assert!(
                    into.can_move(),
                    "{owner} lands nearby, which only a form that can move does"
                ),
                MorphPlacement::Reserve | MorphPlacement::Revalidate => {}
            }
            self.validate_requires(&owner, morph.requires());
            // A time read from a stat the type never declares would silently
            // mean an instant change — the same validates-but-lies class as a
            // cost without its pool.
            if let Quantity::Stat(stat) = morph.time() {
                assert!(
                    def.base_stats.contains_key(&stat),
                    "{owner} reads its time from a stat the type does not carry"
                );
            }
            if let Some(via) = morph.via_type() {
                assert!(
                    via != def.name && via != morph.into_type(),
                    "{owner} wears a form that is one of its own ends"
                );
                let interim = self
                    .entity(via)
                    .unwrap_or_else(|| panic!("{owner} wears a form that is not registered"));
                assert!(
                    !interim.is_remains(),
                    "{owner} wears remains, which only a death may leave"
                );
                // The interim form stands exactly where the origin stood, so
                // entering and leaving it moves nothing on the grid; whether
                // it holds those cells is its own.
                if let (Some(from), Some(worn)) = (def.location, interim.location) {
                    assert!(
                        from.size() == worn.size() && from.occupation() == worn.occupation(),
                        "{owner} wears a form whose footprint differs from its own"
                    );
                }
                // The time is read while the interim form is worn.
                if let Quantity::Stat(stat) = morph.time() {
                    assert!(
                        interim.base_stats.contains_key(&stat),
                        "{owner} reads its time from a stat the form it wears does not carry"
                    );
                }
            }
            for cost in morph.costs() {
                match cost {
                    Cost::Resources(resources) => {
                        for kind in resources.keys() {
                            assert!(
                                self.has_resource(kind),
                                "{owner} costs unregistered resource kind '{kind}'"
                            );
                        }
                    }
                    Cost::Energy(_) => assert!(
                        def.has_energy(),
                        "{owner} has an energy cost but no max_energy stat"
                    ),
                    Cost::Health(_) => assert!(
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
            // Every number a cast reads from a stat is read off whoever casts,
            // so the caster must carry it: a reach it cannot read could never
            // be closed, and a cast it cannot time would never land.
            if let SkillCaster::Entity { reach, casting, .. } = &skill_def.caster {
                let require = |quantity: Quantity, what: &str| {
                    if let Quantity::Stat(stat) = quantity {
                        assert!(
                            def.base_stats.contains_key(&stat),
                            "entity type '{}' has skill '{}' reading its {what} from a stat it does not carry",
                            def.name,
                            self.skill_name(skill).unwrap_or("<unregistered>"),
                        );
                    }
                };
                match reach {
                    Reach::Within(cells) => require(*cells, "reach"),
                    Reach::Wherever => {}
                }
                match casting {
                    Casting::Delayed { point, period } => {
                        require(*point, "cast point");
                        require(*period, "cast period");
                        // Registration held the two to each other by the least
                        // either can fold to; here the carrier's own numbers
                        // are known, so the relation is held to what it would
                        // actually cast at.
                        let authored = |quantity: Quantity| match quantity {
                            Quantity::Constant(value) => Some(FixedU64::from_num(value)),
                            Quantity::Stat(stat) => def.base_stat(stat),
                        };
                        if let (Some(point), Some(period)) = (authored(*point), authored(*period)) {
                            assert!(
                                period >= point,
                                "entity type '{}' casts '{}' over {point} ticks but is freed after {period}",
                                def.name,
                                self.skill_name(skill).unwrap_or("<unregistered>"),
                            );
                        }
                    }
                    Casting::Instant => {}
                }
            }
            for cost in costs {
                match cost {
                    // Kinds were checked when the skill was registered; the
                    // stockpile is the owner's, not the type's, so there is
                    // nothing type-level left to require.
                    Cost::Resources(_) => {}
                    Cost::Energy(_) => assert!(
                        def.has_energy(),
                        "entity type '{}' has skill '{}' with an energy cost but no max_energy stat",
                        def.name,
                        self.skill_name(skill).unwrap_or("<unregistered>"),
                    ),
                    Cost::Health(_) => assert!(
                        def.has_health(),
                        "entity type '{}' has skill '{}' with a health cost but no health pool",
                        def.name,
                        self.skill_name(skill).unwrap_or("<unregistered>"),
                    ),
                }
            }
            // A buff a self-cast puts on the caster draws its upkeep from the
            // caster's own pools, so the caster must have them; a buff cast on
            // something else draws from whatever it lands on, which only the
            // cast can know.
            if let SkillCaster::Entity {
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::ApplyBuff(buff),
                ..
            } = &skill_def.caster
                && let Lasting::Upkeep { costs: upkeep, .. } =
                    &self.entity_buff_defs[buff.index()].lasting
            {
                for cost in upkeep {
                    match cost {
                        Cost::Resources(_) => {}
                        Cost::Energy(_) => assert!(
                            def.has_energy(),
                            "entity type '{}' has skill '{}' keeping up a buff from energy but no max_energy stat",
                            def.name,
                            self.skill_name(skill).unwrap_or("<unregistered>"),
                        ),
                        Cost::Health(_) => assert!(
                            def.has_health(),
                            "entity type '{}' has skill '{}' keeping up a buff from health but no health pool",
                            def.name,
                            self.skill_name(skill).unwrap_or("<unregistered>"),
                        ),
                    }
                }
            }
        }
    }

    /// Checks that every field the type projects, reads for placement, or
    /// answers to was minted by this registry, and that every field-effect
    /// modifier targets a registered entity stat.
    fn validate_fields(&self, def: &EntityTypeDef) {
        let known = |field: FieldId| field.index() < self.field_defs.len();
        for source in &def.field_sources {
            assert!(
                known(source.field()),
                "entity type '{}' projects an unregistered field",
                def.name
            );
            match source.growth() {
                FieldGrowth::Gradual { initial_radius, .. } => assert!(
                    initial_radius <= source.radius(),
                    "entity type '{}' starts a field beyond its radius",
                    def.name
                ),
                FieldGrowth::Instant => {}
            }
            match source.while_constructing() {
                Emission::Full | Emission::Held(_) => assert!(
                    def.build_time.is_some(),
                    "entity type '{}' projects a field while constructing but is never constructed",
                    def.name
                ),
                Emission::Nothing => {}
            }
            for (emission, when) in [
                (source.while_constructing(), "constructing"),
                (source.while_disabled(), "disabled"),
            ] {
                match emission {
                    Emission::Held(reach) => assert!(
                        reach <= source.radius(),
                        "entity type '{}' projects a field beyond its radius while {when}",
                        def.name
                    ),
                    Emission::Full | Emission::Nothing => {}
                }
            }
        }
        for rule in &def.field_placement {
            assert!(
                known(rule.field()),
                "entity type '{}' reads an unregistered field for placement",
                def.name
            );
        }
        for act in &def.on_stand {
            match act {
                StandingAct::Field { field, .. } => assert!(
                    known(*field),
                    "entity type '{}' acts on an unregistered field when it stands",
                    def.name
                ),
            }
        }
        for effect in &def.field_effects {
            assert!(
                known(effect.field()),
                "entity type '{}' answers to an unregistered field",
                def.name
            );
            match effect.kind() {
                EntityEffect::Modifiers(modifiers) => {
                    assert!(
                        !modifiers.is_empty(),
                        "entity type '{}' has a field effect with no modifiers",
                        def.name
                    );
                    for modifier in modifiers {
                        assert!(
                            modifier.stat.index() < self.entity_stats.len(),
                            "entity type '{}' has a field effect on an unregistered entity stat",
                            def.name
                        );
                        // A modifier folds into a stat the instance carries;
                        // one on a stat the type never declares would validate
                        // and do nothing.
                        assert!(
                            def.base_stats.contains_key(&modifier.stat),
                            "entity type '{}' has a field effect on stat '{}', which the type does not carry",
                            def.name,
                            self.entity_stats
                                .iter()
                                .find(|(_, id)| **id == modifier.stat)
                                .map_or("?", |(name, _)| name.as_str())
                        );
                    }
                }
                EntityEffect::Disable | EntityEffect::Conceal => {}
            }
        }
    }

    /// Checks that a detection reveals registered layers only. A blind one
    /// names none.
    fn validate_detection(&self, owner: &str, detection: Detection) {
        match detection {
            Detection::Reveals(layers) => {
                let unregistered = layers & !self.registered_layers();
                assert!(
                    unregistered == LayerMask::EMPTY,
                    "{owner} detects on unregistered layers {unregistered}"
                );
            }
            Detection::Blind => {}
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
    /// the kind it names, and that no name serves as both an entity type and a
    /// tag.
    fn validate_requires(&self, owner: &str, requires: &[Requirement]) {
        for entry in requires {
            match entry {
                Requirement::EntityType(name) => {
                    assert!(
                        self.defs_by_name.contains_key(name),
                        "{owner} requires the entity type '{name}', which is not registered"
                    );
                    assert!(
                        !self.tags.contains(name),
                        "{owner} requires the entity type '{name}', which is also a registered tag"
                    );
                }
                Requirement::Tag(name) => {
                    assert!(
                        self.tags.contains(name),
                        "{owner} requires the tag '{name}', which is not registered"
                    );
                    assert!(
                        !self.defs_by_name.contains_key(name),
                        "{owner} requires the tag '{name}', which is also a registered entity type"
                    );
                }
                Requirement::Research(research) => assert!(
                    research.index() < self.research_defs.len(),
                    "{owner} requires a research this registry never minted"
                ),
                Requirement::Annexed(name) => {
                    let annex = self.entity(name).is_some_and(|def| def.annex.is_some());
                    assert!(
                        annex,
                        "{owner} requires '{name}' docked, which is not a registered annex"
                    );
                }
            }
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
            EntityStatId::LIFETIME,
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
        // value below the floor is a number the type never actually has: the fold
        // raises it to the floor on the first tick. Driven off the registered
        // floors, content's own stats included, so a declaration and the fold
        // cannot disagree.
        for (name, &stat) in &self.entity_stats {
            let floor = self.entity_stat_defs[stat.index()].floor();
            if floor == FixedU64::ZERO {
                continue;
            }
            if let Some(value) = def.base_stat(stat) {
                assert!(
                    value >= floor,
                    "entity type '{}' has {name} below its minimum of {floor}",
                    def.name,
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

    /// Checks that a repair capability is complete and that the terms it
    /// charges by resolve. What it mends is settled once every type is
    /// registered, in [`validate_repairs`](Self::validate_repairs).
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
            RepairCost::PerTick(price) => {
                for kind in price.keys() {
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

        for kind in def.price.keys() {
            check_kind(kind, "price");
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

    /// Checks that every type a death hands on is registered and stands
    /// somewhere, and that what remains rot into bottoms out.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration,
    /// because a type may be registered before what it leaves.
    fn validate_leaves(&self, def: &EntityTypeDef) {
        let Some(dying) = &def.dying else { return };

        for bequest in dying.leaves() {
            let left = bequest.entity_type();
            assert!(
                self.entity(left).is_some(),
                "entity type '{}' leaves '{left}', which is not registered",
                def.name
            );
        }

        // Remains rotting into remains are a decay chain, and a chain that
        // comes back round is a body that never leaves the map.
        if !def.is_remains() {
            return;
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut pending: Vec<&EntityTypeDef> = self.rots_into(def).collect();
        while let Some(next) = pending.pop() {
            assert!(
                next.name != def.name,
                "entity type '{}' rots into a chain that comes back round to '{}'",
                def.name,
                def.name
            );
            if seen.insert(next.name.as_str()) {
                pending.extend(self.rots_into(next));
            }
        }
    }

    /// What one remains type rots into: every remains among what its own death
    /// leaves.
    fn rots_into<'a>(&'a self, def: &'a EntityTypeDef) -> impl Iterator<Item = &'a EntityTypeDef> {
        def.dying
            .iter()
            .flat_map(|dying| dying.leaves())
            .filter_map(|bequest| self.entity(bequest.entity_type()))
            .filter(|left| left.is_remains())
    }

    /// Checks that a type tagged as remains defines only what remains can use:
    /// identity, footprint, tags, how it is picked out, the lifetime it lies
    /// there for, and a dying phase of its own for what it rots into.
    ///
    /// Implemented as an equality check against a minimal definition carrying
    /// only the allowed data, so fields added to [`EntityTypeDef`] later are
    /// refused until somebody decides what remains do with them.
    fn validate_remains(&self, def: &EntityTypeDef) {
        if !def.is_remains() {
            return;
        }

        assert!(
            def.base_stat(EntityStatId::LIFETIME).is_some(),
            "entity type '{}' is remains but carries no lifetime, so it would lie there for good",
            def.name
        );
        assert!(
            def.dying
                .as_ref()
                .is_none_or(|dying| dying.dying_time().is_none()),
            "entity type '{}' is remains and states a dying time: a body has lain its whole life already, so it goes the tick its decay ends",
            def.name
        );

        let mut allowed = EntityTypeDef::new(def.name.clone());
        allowed.race = def.race.clone();
        allowed.location = def.location;
        allowed.dying = def.dying.clone();
        allowed.tags = def.tags.clone();
        allowed.selection = def.selection.clone();
        allowed = allowed.with_stat(
            EntityStatId::LIFETIME,
            def.base_stat(EntityStatId::LIFETIME)
                .expect("the lifetime was just required"),
        );

        assert_eq!(
            *def, allowed,
            "entity type '{}' is remains, but defines live-gameplay data that remains never use",
            def.name
        );
    }

    /// Checks that every type and tag a cast's aim names is registered.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration,
    /// because a skill is registered before the type that carries it, and may
    /// well name that type.
    fn validate_aim(&self, name: &str, def: &SkillDef) {
        let SkillCaster::Entity { target, .. } = &def.caster else {
            return;
        };
        match target {
            EntityCastTarget::Standing { kinds, .. } | EntityCastTarget::Fallen { kinds } => {
                self.validate_kinds(&format!("skill '{name}'"), "aims at", kinds);
            }
            EntityCastTarget::Caster | EntityCastTarget::Position => {}
        }
    }

    /// Checks that every type and tag a repairer names is registered.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration,
    /// because a type may mend one registered after it — or itself.
    fn validate_repairs(&self, def: &EntityTypeDef) {
        let Some(repairer) = &def.repairer else {
            return;
        };

        self.validate_kinds(
            &format!("entity type '{}'", def.name),
            "repairs",
            repairer.repairs(),
        );
    }

    /// Checks that every admission-list entry resolves to a registered entity
    /// type or tag. Carried types may register after their carrier, so this
    /// runs in the deferred pass.
    fn validate_carries(&self, def: &EntityTypeDef) {
        let Some(transporter) = &def.transporter else {
            return;
        };

        self.validate_kinds(
            &format!("entity type '{}'", def.name),
            "carries",
            transporter.carries(),
        );
    }

    /// Every registered type a filter names — one it lists, or one wearing a
    /// tag it lists.
    fn matching<'a>(&'a self, kinds: &'a Kinds) -> impl Iterator<Item = &'a EntityTypeDef> {
        self.defs.iter().filter(move |def| kinds.admits(def))
    }

    /// Every registered type one entry of a filter names.
    fn matching_one<'a>(&'a self, kind: &'a Kind) -> impl Iterator<Item = &'a EntityTypeDef> {
        self.defs.iter().filter(move |def| kind.names(def))
    }

    /// Checks that every name a filter lists is something this registry knows:
    /// a registered entity type, or a registered tag.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration,
    /// because a filter may name a type registered after the one that declares
    /// it.
    fn validate_kinds(&self, owner: &str, what: &str, kinds: &Kinds) {
        // The constructor refuses a filter that names nobody; one written past
        // it would quietly admit nothing at all.
        if let Kinds::Only(named) = kinds {
            assert!(!named.is_empty(), "{owner} {what} nothing at all");
        }
        for kind in kinds.kinds() {
            let known = match kind {
                Kind::Type(name) => self.defs_by_name.contains_key(name),
                Kind::Tag(tag) => self.tags.contains(tag),
            };
            assert!(
                known,
                "{owner} {what} {}, which is not registered",
                kind.describe()
            );
        }
    }

    /// Checks that no presence the definition declares admits nobody.
    fn validate_crews(&self, def: &EntityTypeDef) {
        let mut presences: Vec<&WorkPresence> = Vec::new();
        if let Some(builder) = &def.builder
            && let BuilderAttendance::Crew(presence) = builder.attendance()
        {
            presences.push(presence);
        }
        if let Some(repairer) = &def.repairer {
            presences.push(repairer.presence());
        }
        if let Some(carrier) = &def.resource_carrier {
            for kind in carrier.kinds() {
                presences.push(
                    carrier
                        .harvest_data(kind)
                        .expect("a carrier's kinds are the keys of its catalogue")
                        .presence(),
                );
            }
        }
        for presence in presences {
            let admits_nobody = match presence.crewing() {
                Crewing::Counted(at_once) => at_once.admits_nobody(),
                Crewing::Seated => false,
            };
            assert!(
                !admits_nobody,
                "entity type '{}' declares a crew limit of nobody",
                def.name
            );
        }
    }

    /// Checks that every attachment a type's capabilities declare names a
    /// berth group the jobs it reaches offer: every type a builder raises, at
    /// least one source of a kind a carrier attaches for, and at least one type
    /// a repairer mends.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because
    /// a worker may be registered before the jobs it attaches to.
    fn validate_berths(&self, def: &EntityTypeDef) {
        let offers = |job: &EntityTypeDef, group: &str| {
            job.berths
                .as_ref()
                .is_some_and(|berths| berths.group(group).is_some())
        };

        if let Some(builder) = &def.builder
            && let BuilderAttendance::Crew(WorkPresence::Attached(attachment)) =
                builder.attendance()
        {
            for type_name in builder.builds() {
                assert!(
                    self.entity(type_name)
                        .is_some_and(|built| offers(built, attachment.berths())),
                    "entity type '{}' attaches to berth group '{}' of '{type_name}', which declares no such group",
                    def.name,
                    attachment.berths()
                );
            }
        }
        if let Some(carrier) = &def.resource_carrier {
            for kind in carrier.kinds() {
                let data = carrier
                    .harvest_data(kind)
                    .expect("a carrier's kinds are the keys of its catalogue");
                if let WorkPresence::Attached(attachment) = data.presence() {
                    // Only the sources the carrier's kind admits count: a group
                    // offered by a seam it may not work is a group it can never
                    // sit in.
                    let offered = self.entities().any(|source| {
                        source
                            .resource_source
                            .as_ref()
                            .is_some_and(|resource| resource.kind() == kind)
                            && data.sources().admits(source)
                            && offers(source, attachment.berths())
                    });
                    assert!(
                        offered,
                        "entity type '{}' attaches to berth group '{}' of {kind} sources, and no {kind} source it may work declares such a group",
                        def.name,
                        attachment.berths()
                    );
                }
            }
        }
        if let Some(repairer) = &def.repairer
            && let WorkPresence::Attached(attachment) = repairer.presence()
        {
            let offered = self.entities().any(|target| {
                repairer.repairs().admits(target) && offers(target, attachment.berths())
            });
            assert!(
                offered,
                "entity type '{}' attaches to berth group '{}' of what it mends, and no registered type it mends declares such a group",
                def.name,
                attachment.berths()
            );
        }
    }

    /// Checks what a breeder declares: the type it breeds is registered, stands
    /// somewhere, is not the breeder itself and is a broodling that sits in a
    /// group the breeder offers, with at least the limit in slots, and is one
    /// cell across; a period read from a stat names one the breeder carries.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because
    /// a breeder may be registered before what it breeds.
    fn validate_breeder(&self, def: &EntityTypeDef) {
        let Some(brood) = &def.breeder else {
            return;
        };
        let owner = format!("entity type '{}' breeding '{}'", def.name, brood.breeds());
        assert!(brood.breeds() != def.name, "{owner} breeds its own type");
        let bred = self
            .entity(brood.breeds())
            .unwrap_or_else(|| panic!("{owner} names a type that is not registered"));
        let location = bred
            .location
            .unwrap_or_else(|| panic!("{owner} names a type with no location"));
        assert!(!bred.can_move(), "{owner} names a type that can move");
        let attachment = bred
            .broodling
            .as_ref()
            .map(BroodlingDef::attachment)
            .unwrap_or_else(|| panic!("{owner} names a type that is no broodling"));
        let group = def
            .berths
            .as_ref()
            .and_then(|berths| berths.group(attachment.berths()))
            .unwrap_or_else(|| {
                panic!(
                    "{owner} seats it in berth group '{}', which the breeder does not declare",
                    attachment.berths()
                )
            });
        assert!(
            brood.limit() <= group.slots(),
            "{owner} keeps up to {} but berth group '{}' seats only {}",
            brood.limit(),
            attachment.berths(),
            group.slots()
        );
        assert!(
            location.size() == CellSize::ONE,
            "{owner} seats a type wider than one cell in a berth"
        );
        if let Quantity::Stat(stat) = brood.period() {
            assert!(
                def.base_stats.contains_key(&stat),
                "{owner} reads its period from a stat the type does not carry"
            );
        }
    }

    /// Checks that a broodling type is bred by something: at least one
    /// registered breeder breeds it.
    ///
    /// Runs in [`validate`](Self::validate) rather than at registration, because
    /// the breeder may be registered after what it breeds.
    fn validate_broodling(&self, def: &EntityTypeDef) {
        if def.broodling.is_none() {
            return;
        }
        assert!(
            self.entities().any(|breeder| {
                breeder
                    .breeder
                    .as_ref()
                    .is_some_and(|brood| brood.breeds() == def.name)
            }),
            "entity type '{}' is a broodling, and no registered type breeds it",
            def.name
        );
    }

    /// Checks that every source a carrier's kind restricts itself to is a
    /// registered source of that kind.
    fn validate_harvest_sources(&self, def: &EntityTypeDef) {
        let Some(carrier) = &def.resource_carrier else {
            return;
        };
        for kind in carrier.kinds() {
            let data = carrier
                .harvest_data(kind)
                .expect("a carrier's kinds are the keys of its catalogue");
            // Every name must be something this registry knows, and must
            // reach a source of the kind being carried: a type that yields it,
            // or a tag some such type wears.
            let owner = format!("entity type '{}'", def.name);
            self.validate_kinds(&owner, &format!("harvests {kind} from"), data.sources());
            for named in data.sources().kinds() {
                let reaches_a_source = self.matching_one(named).any(|source| {
                    source
                        .resource_source
                        .as_ref()
                        .is_some_and(|resource| resource.kind() == kind)
                });
                assert!(
                    reaches_a_source,
                    "{owner} harvests {kind} from '{}', which is no {kind} source",
                    named.name()
                );
            }
        }
    }

    /// Checks that a type's docks stand clear of its own footprint and of each
    /// other, take registered annexes, and belong to a primary that stands
    /// still, can raise what it takes and attends the work from where it stands.
    fn validate_docks(&self, def: &EntityTypeDef) {
        let size = def
            .location
            .expect("validated content defines a location")
            .size();
        // The bond is derived from where the primary stands, so a dock holds
        // still only if its primary does: a moving primary's anchor jumps a
        // whole cell whenever its position crosses a boundary.
        assert!(
            def.docks.is_empty() || def.base_stat(EntityStatId::SPEED).is_none(),
            "entity type '{}' offers docks but carries the speed stat",
            def.name
        );

        // A primary raises its annex from where it stands, so it may attend the
        // site or leave it to itself — but it cannot go inside the work, be
        // consumed by it, or sit in its berths: each would take the primary off
        // the dock it is filling.
        // A type with no builder at all is answered for per dock below.
        if let Some(builder) = &def.builder
            && !def.docks.is_empty()
        {
            let attends = builder.attendance();
            assert!(
                matches!(
                    attends,
                    BuilderAttendance::Crew(WorkPresence::Present { .. })
                        | BuilderAttendance::Unattended
                ),
                "entity type '{}' offers docks but raises its sites as {}",
                def.name,
                match attends {
                    BuilderAttendance::Crew(WorkPresence::Hidden { .. }) => "a hidden builder",
                    BuilderAttendance::Crew(WorkPresence::Attached(_)) => "an attached builder",
                    BuilderAttendance::Consumed => "a builder consumed by its work",
                    BuilderAttendance::Crew(WorkPresence::Present { .. })
                    | BuilderAttendance::Unattended => unreachable!("admitted above"),
                }
            );
        }

        let mut anchors: BTreeSet<CellPos> = BTreeSet::new();
        // Whatever stands in one dock must leave every other dock free to be
        // filled, so the footprints an annex could take are compared and not
        // only the cells their anchors sit on. The types one dock accepts are
        // alternatives, so only different docks are held against each other.
        let mut taken: Vec<(CellRect, &str)> = Vec::new();
        for dock in &def.docks {
            assert!(
                anchors.insert(dock.at()),
                "entity type '{}' offers two docks at ({}, {})",
                def.name,
                dock.at().x,
                dock.at().y
            );
            let at = dock.at();
            assert!(
                at.x >= size.width || at.y >= size.height,
                "entity type '{}' offers a dock at ({}, {}), inside its own footprint",
                def.name,
                at.x,
                at.y
            );

            self.validate_kinds(
                &format!("entity type '{}'", def.name),
                "docks",
                dock.accepts(),
            );
            // A type the filter names outright must be an annex: naming one
            // that could never dock is a mistake content can see. A tag or
            // `any` takes whatever among them is an annex and leaves the rest
            // standing, since otherwise "any annex" could not be said at all.
            if let Kinds::Only(named) = dock.accepts() {
                for kind in named {
                    let Kind::Type(type_name) = kind else {
                        continue;
                    };
                    assert!(
                        self.entity(type_name)
                            .is_some_and(|docked| docked.annex.is_some()),
                        "entity type '{}' docks '{type_name}', which is not an annex",
                        def.name
                    );
                }
            }
            let mut fillable = false;
            let mut this_dock: Vec<(CellRect, &str)> = Vec::new();
            for annex in self
                .matching(dock.accepts())
                .filter(|annex| annex.annex.is_some())
            {
                fillable = true;
                let type_name = annex.name.as_str();
                let annex_size = annex
                    .location
                    .expect("validated content defines a location")
                    .size();
                let rect = CellRect::new(dock.at(), annex_size);
                if let Some((_, other)) = taken
                    .iter()
                    .find(|&&(standing, _)| standing.intersects(rect))
                {
                    panic!(
                        "entity type '{}' docks '{type_name}' at ({}, {}), over the ground '{other}' stands on",
                        def.name,
                        dock.at().x,
                        dock.at().y
                    );
                }
                this_dock.push((rect, type_name));

                assert!(
                    def.builder
                        .as_ref()
                        .is_some_and(|builder| builder.can_build(type_name)),
                    "entity type '{}' docks '{type_name}' but cannot raise it",
                    def.name
                );
            }
            assert!(
                fillable,
                "entity type '{}' offers a dock at ({}, {}) that names no annex",
                def.name,
                dock.at().x,
                dock.at().y
            );
            taken.extend(this_dock);
        }
    }

    /// Checks that an annex is a constructible building that holds the cells it
    /// stands on, carries the stat any fade it declares moves, and fits some
    /// registered dock.
    fn validate_annex(&self, def: &EntityTypeDef) {
        let Some(annex) = def.annex else { return };
        assert!(
            def.build_time.is_some(),
            "annex '{}' is not constructible",
            def.name
        );
        // A modifier moves only a stat its type carries, so a fade the
        // type never declared would drain nothing at all.
        if let AloneConduct::Standing {
            life: AnnexLife::Fades { .. },
            ..
        } = annex.alone()
        {
            assert!(
                def.base_stat(EntityStatId::HEALTH_DRAIN).is_some(),
                "annex '{}' fades but does not carry the health_drain stat",
                def.name
            );
        }
        assert!(
            def.base_stat(EntityStatId::SPEED).is_none(),
            "annex '{}' carries the speed stat",
            def.name
        );
        assert!(
            def.location
                .expect("validated content defines a location")
                .solidity()
                .claims_cells(),
            "annex '{}' does not claim the cells it stands on",
            def.name
        );
        assert!(def.has_health(), "annex '{}' has no health pool", def.name);
        let docked = self
            .defs
            .iter()
            .any(|primary| primary.docks.iter().any(|dock| dock.accepts().admits(def)));
        assert!(docked, "annex '{}' fits no registered dock", def.name);
    }

    /// Checks that a type raised over a resource source names a registered
    /// source of the kind the type itself yields, on the same footprint and the
    /// same layers, and is constructible.
    fn validate_overbuilds(&self, def: &EntityTypeDef) {
        let Some(over) = &def.overbuilds else { return };
        let source = self.entity(over).unwrap_or_else(|| {
            panic!(
                "entity type '{}' overbuilds '{over}', which is not registered",
                def.name
            )
        });
        let yielded = source.resource_source.as_ref().unwrap_or_else(|| {
            panic!(
                "entity type '{}' overbuilds '{over}', which is not a resource source",
                def.name
            )
        });
        let own = def.resource_source.as_ref().unwrap_or_else(|| {
            panic!(
                "entity type '{}' overbuilds '{over}' but is not a resource source itself",
                def.name
            )
        });
        assert!(
            own.kind() == yielded.kind(),
            "entity type '{}' yields {} but overbuilds '{over}', which yields {}",
            def.name,
            own.kind(),
            yielded.kind()
        );
        assert!(
            def.location.map(|location| location.size())
                == source.location.map(|location| location.size()),
            "entity type '{}' overbuilds '{over}' on a footprint of another size",
            def.name
        );
        assert!(
            def.location.map(|location| location.occupation())
                == source.location.map(|location| location.occupation()),
            "entity type '{}' overbuilds '{over}', which occupies other layers",
            def.name
        );
        assert!(
            def.build_time.is_some(),
            "entity type '{}' overbuilds '{over}' but is not constructible",
            def.name
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
