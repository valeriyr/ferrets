//! The crew a job records while workers are on it.
//!
//! A job marks itself with the workers attending it, so who is on it is a lookup on
//! the job rather than a sweep over every worker in the world. This module owns the
//! membership arithmetic and the one question the orders ask of a crew: whether it
//! shuts a newcomer out.

use std::collections::BTreeSet;

use bevy_ecs::{
    component::{Component, Mutable},
    entity::Entity,
    world::World,
};

use crate::{
    components::{
        build::UnderConstructionComponent, repair::UnderRepairComponent,
        resource::UnderHarvestComponent, transport::TransporterComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    simulation_id::SimulationId,
};

/// A component that holds the workers attending the entity carrying it.
pub(super) trait Crew: Component<Mutability = Mutable> + Default {
    /// The workers on the job.
    fn members(&self) -> &BTreeSet<SimulationId>;
    /// The workers on the job, to add to or remove from.
    fn members_mut(&mut self) -> &mut BTreeSet<SimulationId>;
}

impl Crew for UnderHarvestComponent {
    fn members(&self) -> &BTreeSet<SimulationId> {
        &self.carriers
    }

    fn members_mut(&mut self) -> &mut BTreeSet<SimulationId> {
        &mut self.carriers
    }
}

impl Crew for UnderConstructionComponent {
    fn members(&self) -> &BTreeSet<SimulationId> {
        &self.builders
    }

    fn members_mut(&mut self) -> &mut BTreeSet<SimulationId> {
        &mut self.builders
    }
}

impl Crew for UnderRepairComponent {
    fn members(&self) -> &BTreeSet<SimulationId> {
        &self.repairers
    }

    fn members_mut(&mut self) -> &mut BTreeSet<SimulationId> {
        &mut self.repairers
    }
}

impl Crew for TransporterComponent {
    fn members(&self) -> &BTreeSet<SimulationId> {
        &self.passengers
    }

    fn members_mut(&mut self) -> &mut BTreeSet<SimulationId> {
        &mut self.passengers
    }
}

/// Adds `member` to the crew on `job`, marking a job nobody was on yet.
pub(super) fn join<C: Crew>(world: &mut World, job: Entity, member: Entity) {
    let id = entity_def::simulation_id(world, member);
    let mut job_mut = world.entity_mut(job);

    match job_mut.get_mut::<C>() {
        Some(mut crew) => {
            crew.members_mut().insert(id);
        }
        None => {
            let mut crew = C::default();
            crew.members_mut().insert(id);
            job_mut.insert(crew);
        }
    }
}

/// Adds `member` to a crew the job already carries, and leaves a job carrying none
/// alone.
///
/// For a crew that rides along on a component saying something else as well — a
/// site's construction progress — where conjuring the component up would claim
/// something about the job that is not this module's to claim.
pub(super) fn join_existing<C: Crew>(world: &mut World, job: Entity, member: Entity) {
    let id = entity_def::simulation_id(world, member);

    if let Some(mut crew) = world.entity_mut(job).get_mut::<C>() {
        crew.members_mut().insert(id);
    }
}

/// Removes `member` from the crew on `job`, and reports whether that leaves the job
/// unmanned.
///
/// A job carrying no crew reports `false`: nobody is the last one out of a crew that
/// does not exist.
pub(super) fn leave<C: Crew>(world: &mut World, job: Entity, member: Entity) -> bool {
    let id = entity_def::simulation_id(world, member);
    let mut job_mut = world.entity_mut(job);

    let Some(mut crew) = job_mut.get_mut::<C>() else {
        return false;
    };
    crew.members_mut().remove(&id);
    crew.members().is_empty()
}

/// Like [`leave`], but the last worker out takes the marker with it — for a crew whose
/// component says nothing beyond "somebody is on this".
pub(super) fn leave_and_unmark<C: Crew>(world: &mut World, job: Entity, member: Entity) {
    if leave::<C>(world, job, member) {
        world.entity_mut(job).remove::<C>();
    }
}

/// Whether the crew on `job` shuts `newcomer` out of it, given what `shares` says
/// about a worker's willingness to share this kind of job.
///
/// A job is exclusive when either side declines to share it, so a lone worker turns
/// every newcomer away and a stacking crew still yields to one that works alone.
/// What being shut out means for the newcomer's order — waiting in place for the
/// crew to clear, or giving the job up — is the caller's to decide.
pub(super) fn excludes<C: Crew>(
    world: &World,
    job: Entity,
    newcomer: Entity,
    shares: impl Fn(&World, Entity) -> bool,
) -> bool {
    let Some(crew) = world.entity(job).get::<C>() else {
        return false;
    };
    let newcomer_shares = shares(world, newcomer);

    crew.members()
        .iter()
        .filter_map(|&id| world.resource::<EntityIndex>().alive(id))
        .filter(|&other| other != newcomer)
        .any(|other| !newcomer_shares || !shares(world, other))
}
