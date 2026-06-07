//! Registry of all content-defined entity types.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use super::entity_type_def::EntityTypeDef;

/// Stores every [`EntityTypeDef`], keyed by type name.
#[derive(Resource, Default)]
pub struct ContentRegistry {
    entities: HashMap<String, EntityTypeDef>,
}

impl ContentRegistry {
    /// Registers an entity type definition, replacing any existing entry with the same name.
    pub fn register(&mut self, def: EntityTypeDef) {
        self.entities.insert(def.name.clone(), def);
    }

    /// Returns the definition for the given type name, or `None` if not registered.
    pub fn entity(&self, name: &str) -> Option<&EntityTypeDef> {
        self.entities.get(name)
    }
}
