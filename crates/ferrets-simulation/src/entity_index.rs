//! Lookup of simulation entities by [`SimulationId`], partitioned by lifecycle stage.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::{components::hidden::HiddenComponent, simulation_id::SimulationId};

/// Maps every [`SimulationId`] to its [`Entity`], split into alive and dying sets.
///
/// An entity is registered at spawn — as alive, or directly as dying for
/// corpse-like remains — moved from alive to dying when destroyed, and removed
/// when its dying phase completes. Iteration order is ascending by
/// [`SimulationId`], so per-entity processing stays deterministic across peers.
#[derive(Resource, Default)]
pub struct EntityIndex {
    alive: BTreeMap<SimulationId, Entity>,
    dying: BTreeMap<SimulationId, Entity>,
}

impl EntityIndex {
    /// Registers a newly spawned entity as alive.
    pub fn insert_alive(&mut self, id: SimulationId, entity: Entity) {
        self.alive.insert(id, entity);
    }

    /// Registers a newly spawned entity that begins its life dying (a corpse).
    pub fn insert_dying(&mut self, id: SimulationId, entity: Entity) {
        self.dying.insert(id, entity);
    }

    /// Returns the alive entity with the given id, or `None` if it is dying or gone.
    pub fn alive(&self, id: SimulationId) -> Option<Entity> {
        self.alive.get(&id).copied()
    }

    /// Returns the alive, on-map entity with the given id — `None` if it is
    /// dying, hidden, or gone.
    pub fn interactable(&self, world: &World, id: SimulationId) -> Option<Entity> {
        let entity = self.alive(id)?;
        if world.entity(entity).contains::<HiddenComponent>() {
            None
        } else {
            Some(entity)
        }
    }

    /// Moves an entity from the alive set to the dying set.
    ///
    /// No-op if the id is not in the alive set.
    pub fn mark_dying(&mut self, id: SimulationId) {
        if let Some(entity) = self.alive.remove(&id) {
            self.dying.insert(id, entity);
        }
    }

    /// Removes an entity from the dying set once its dying phase has completed.
    pub fn remove_dying(&mut self, id: SimulationId) {
        self.dying.remove(&id);
    }

    /// Returns all alive entities with their ids in ascending [`SimulationId`] order.
    pub fn alive_entries(&self) -> Vec<(SimulationId, Entity)> {
        self.alive.iter().map(|(&id, &e)| (id, e)).collect()
    }

    /// Returns all dying entities with their ids in ascending [`SimulationId`] order.
    pub fn dying_entries(&self) -> Vec<(SimulationId, Entity)> {
        self.dying.iter().map(|(&id, &e)| (id, e)).collect()
    }

    /// Returns every simulation entity — alive, then dying — with their ids,
    /// each group in ascending [`SimulationId`] order.
    pub fn all_entries(&self) -> Vec<(SimulationId, Entity)> {
        let mut entries = self.alive_entries();
        entries.extend(self.dying_entries());
        entries
    }
}
