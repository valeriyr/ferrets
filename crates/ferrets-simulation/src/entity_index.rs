//! Lookup of simulation entities by [`SimulationId`], partitioned by lifecycle stage.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::{components::hidden::HiddenComponent, simulation_id::SimulationId};

/// Maps every [`SimulationId`] to its [`Entity`], split into alive and dying
/// sets, with the remains among the dying kept in a set of their own.
///
/// An entity is registered at spawn — as alive, or as a body already dying —
/// moved from alive to dying when destroyed, and removed when its dying phase
/// completes. A body leaves the remains the moment its decay runs out, while
/// it goes on finishing that death. Iteration order is ascending by
/// [`SimulationId`], so per-entity processing stays deterministic across peers.
#[derive(Resource, Default)]
pub struct EntityIndex {
    alive: BTreeMap<SimulationId, Entity>,
    dying: BTreeMap<SimulationId, Entity>,
    remains: BTreeMap<SimulationId, Entity>,
}

impl EntityIndex {
    /// Returns the alive entity with the given id, or `None` if it is dying or gone.
    pub fn alive(&self, id: SimulationId) -> Option<Entity> {
        self.alive.get(&id).copied()
    }

    /// Returns the remains with the given id, or `None` if the id names
    /// anything else — the living, the dying, or what is already gone.
    pub fn remains(&self, id: SimulationId) -> Option<Entity> {
        self.remains.get(&id).copied()
    }

    /// Returns the entity with the given id whatever stage it is at, or `None`
    /// once it is gone.
    ///
    /// A tick can name something that is already dying, and remains begin their
    /// life that way.
    pub fn any(&self, id: SimulationId) -> Option<Entity> {
        self.alive.get(&id).or_else(|| self.dying.get(&id)).copied()
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

    /// Returns all alive entities with their ids in ascending [`SimulationId`] order.
    pub fn alive_entries(&self) -> Vec<(SimulationId, Entity)> {
        self.alive.iter().map(|(&id, &e)| (id, e)).collect()
    }

    /// Returns all dying entities with their ids in ascending [`SimulationId`] order.
    pub fn dying_entries(&self) -> Vec<(SimulationId, Entity)> {
        self.dying.iter().map(|(&id, &e)| (id, e)).collect()
    }

    /// How many remains lie on the map.
    pub fn remains_count(&self) -> usize {
        self.remains.len()
    }

    /// Returns every remains on the map with their ids in ascending
    /// [`SimulationId`] order — which is oldest first, since ids are minted in
    /// order.
    pub fn remains_entries(&self) -> Vec<(SimulationId, Entity)> {
        self.remains.iter().map(|(&id, &e)| (id, e)).collect()
    }

    /// Returns every simulation entity — alive, then dying — with their ids,
    /// each group in ascending [`SimulationId`] order.
    pub fn all_entries(&self) -> Vec<(SimulationId, Entity)> {
        let mut entries = self.alive_entries();
        entries.extend(self.dying_entries());
        entries
    }

    /// Registers a newly spawned entity as alive.
    ///
    /// Panics if the id is already known: ids are minted once and never reused.
    pub fn insert_alive(&mut self, id: SimulationId, entity: Entity) {
        assert!(self.unknown(id), "simulation ids are minted once: {id:?}");
        self.alive.insert(id, entity);
    }

    /// Registers a newly spawned body: one that begins its life dying, and
    /// lies in the remains a cast can aim at and a cap counts.
    ///
    /// Remains are dying entities, so it joins both sets — as
    /// [`remove_dying`](Self::remove_dying) takes it out of both.
    ///
    /// Panics if the id is already known: ids are minted once and never reused.
    pub fn insert_remains(&mut self, id: SimulationId, entity: Entity) {
        assert!(self.unknown(id), "simulation ids are minted once: {id:?}");
        self.dying.insert(id, entity);
        self.remains.insert(id, entity);
    }

    /// Moves an entity from the alive set to the dying set.
    ///
    /// Panics if the id names nothing alive: only the living begin to die.
    pub fn mark_dying(&mut self, id: SimulationId) {
        let entity = self
            .alive
            .remove(&id)
            .unwrap_or_else(|| panic!("only an alive entity begins to die: {id:?}"));
        self.dying.insert(id, entity);
    }

    /// Removes an entity from the dying set once its dying phase has
    /// completed. A body leaves the remains set with it, as
    /// [`insert_remains`](Self::insert_remains) put it in both.
    ///
    /// Panics if the id names nothing dying: what is still alive leaves the
    /// index by dying first, and what has left cannot leave twice.
    pub fn remove_dying(&mut self, id: SimulationId) {
        assert!(
            !self.alive.contains_key(&id),
            "an alive entity leaves the index by dying first: {id:?}"
        );
        assert!(
            self.dying.contains_key(&id),
            "only a dying entity leaves the index: {id:?}"
        );
        self.dying.remove(&id);
        self.remains.remove(&id);
    }

    /// Takes a body out of the remains set while it goes on finishing its
    /// death, so nothing may aim at it and no cap counts it.
    ///
    /// No-op for an id that names no remains: every dying entity passes
    /// through here, and most were never bodies.
    ///
    /// Panics if the id names nothing dying: a body stops being one in the
    /// middle of the death it is already dying.
    pub fn remove_remains(&mut self, id: SimulationId) {
        assert!(
            self.dying.contains_key(&id),
            "a body stops being one while it is still dying: {id:?}"
        );
        self.remains.remove(&id);
    }

    /// Whether the index has never heard of this id.
    fn unknown(&self, id: SimulationId) -> bool {
        !self.alive.contains_key(&id) && !self.dying.contains_key(&id)
    }
}
