//! Registry of all content-defined entity types and resource kinds.

use std::collections::{BTreeSet, HashMap};

use bevy_ecs::prelude::*;

use super::entity_type_def::EntityTypeDef;

/// Stores every [`EntityTypeDef`], keyed by type name, and every resource kind.
#[derive(Resource, Default)]
pub struct ContentRegistry {
    entities: HashMap<String, EntityTypeDef>,
    resources: BTreeSet<String>,
}

impl ContentRegistry {
    /// Registers an entity type definition.
    ///
    /// Validates the definition against the content already registered, so every
    /// type it references must be registered first: trained and built types,
    /// corpse types, and resource kinds. Registration is final — a type cannot
    /// be replaced — so a validated definition stays consistent and corpse cycles
    /// are unconstructible (a cycle has no member that can be registered first).
    ///
    /// Panics if a type with the same name is already registered, or if the
    /// definition has no location, references an unregistered resource kind,
    /// trains a type that is not a registered trainable type, builds a type that
    /// is not a registered constructible type, or leaves a corpse type that is
    /// unregistered, has no dying phase, or defines live-gameplay data.
    pub fn register(&mut self, def: EntityTypeDef) {
        assert!(
            !self.entities.contains_key(&def.name),
            "entity type '{}' is already registered",
            def.name
        );

        self.validate_location(&def);
        self.validate_resource_kinds(&def);
        self.validate_trains(&def);
        self.validate_builds(&def);
        self.validate_corpse(&def);

        self.entities.insert(def.name.clone(), def);
    }

    /// Returns the definition for the given type name, or `None` if not registered.
    pub fn entity(&self, name: &str) -> Option<&EntityTypeDef> {
        self.entities.get(name)
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

    /// Checks that the definition has the mandatory location properties.
    fn validate_location(&self, def: &EntityTypeDef) {
        assert!(
            def.location.is_some(),
            "entity type '{}' has no location",
            def.name
        );
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

    /// Checks that every type in the definition's train catalogue is a
    /// registered trainable type.
    fn validate_trains(&self, def: &EntityTypeDef) {
        let Some(trainer) = &def.trainer else { return };

        for type_name in trainer.trains() {
            let trainable = self
                .entities
                .get(type_name)
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
            self.entities.contains_key(corpse_type),
            "entity type '{}' leaves an unregistered corpse type '{corpse_type}'",
            def.name
        );
        assert!(
            self.entities[corpse_type].dying.is_some(),
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
        let corpse = &self.entities[corpse_type];

        let mut allowed = EntityTypeDef::new(corpse.name.clone());
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
                .entities
                .get(type_name)
                .is_some_and(|built| built.build_time.is_some());
            assert!(
                constructible,
                "entity type '{}' builds '{type_name}', which is not a registered constructible type",
                def.name
            );
        }
    }
}
