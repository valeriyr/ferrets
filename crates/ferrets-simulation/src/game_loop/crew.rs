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
        build::{SiteWork, UnderConstructionComponent},
        repair::UnderRepairComponent,
        resource::UnderHarvestComponent,
        transport::TransporterComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    simulation_id::SimulationId,
};

/// The invariant every joiner relies on: a job that takes no crew is shut to
/// newcomers before anyone tries to join it.
const NO_CREW_TO_JOIN: &str = "a job that takes no crew admits nobody, so nobody is joined to it";

/// A component that holds the workers attending the entity carrying it.
pub(super) trait Crew: Component<Mutability = Mutable> {
    /// The workers on the job, or `None` for a job that takes no crew.
    fn members(&self) -> Option<&BTreeSet<SimulationId>>;
    /// The workers on the job, to add to or remove from, or `None` for a job
    /// that takes no crew.
    fn members_mut(&mut self) -> Option<&mut BTreeSet<SimulationId>>;
}

impl Crew for UnderHarvestComponent {
    fn members(&self) -> Option<&BTreeSet<SimulationId>> {
        Some(&self.carriers)
    }

    fn members_mut(&mut self) -> Option<&mut BTreeSet<SimulationId>> {
        Some(&mut self.carriers)
    }
}

impl Crew for UnderConstructionComponent {
    fn members(&self) -> Option<&BTreeSet<SimulationId>> {
        match &self.work {
            SiteWork::Crew { builders } => Some(builders),
            SiteWork::Unattended { .. } => None,
        }
    }

    fn members_mut(&mut self) -> Option<&mut BTreeSet<SimulationId>> {
        match &mut self.work {
            SiteWork::Crew { builders } => Some(builders),
            SiteWork::Unattended { .. } => None,
        }
    }
}

impl Crew for UnderRepairComponent {
    fn members(&self) -> Option<&BTreeSet<SimulationId>> {
        Some(&self.repairers)
    }

    fn members_mut(&mut self) -> Option<&mut BTreeSet<SimulationId>> {
        Some(&mut self.repairers)
    }
}

impl Crew for TransporterComponent {
    fn members(&self) -> Option<&BTreeSet<SimulationId>> {
        Some(&self.passengers)
    }

    fn members_mut(&mut self) -> Option<&mut BTreeSet<SimulationId>> {
        Some(&mut self.passengers)
    }
}

/// Adds `member` to the crew on `job`, marking a job nobody was on yet.
///
/// Panics if the job takes no crew: nothing is ever joined to such a job.
pub(super) fn join<C: Crew + Default>(world: &mut World, job: Entity, member: Entity) {
    let id = entity_def::simulation_id(world, member);
    let mut job_mut = world.entity_mut(job);

    match job_mut.get_mut::<C>() {
        Some(mut crew) => {
            crew.members_mut().expect(NO_CREW_TO_JOIN).insert(id);
        }
        None => {
            let mut crew = C::default();
            crew.members_mut().expect(NO_CREW_TO_JOIN).insert(id);
            job_mut.insert(crew);
        }
    }
}

/// Adds `member` to the crew the job already carries.
///
/// For a crew that rides along on a component saying something else as well — a
/// site's construction progress — where conjuring the component up would claim
/// something about the job that is not this module's to claim.
///
/// Panics if the job carries no component, or takes no crew: a caller joins
/// only a job it has just found under way and open to it.
pub(super) fn join_existing<C: Crew>(world: &mut World, job: Entity, member: Entity) {
    let id = entity_def::simulation_id(world, member);

    world
        .entity_mut(job)
        .get_mut::<C>()
        .expect("a member joins only a job it has just found under way")
        .members_mut()
        .expect(NO_CREW_TO_JOIN)
        .insert(id);
}

/// What leaving a job came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Departure {
    /// The member left, and others remain on the job.
    OthersRemain,
    /// The member was the last one out: the job stands unmanned.
    LastOut,
    /// The job no longer carries the component: it finished or was torn down
    /// before the member's order noticed.
    JobGone,
}

/// Removes `member` from the crew on `job`.
///
/// Panics if the job takes no crew, or the member was not on it: every member
/// joined before it leaves.
pub(super) fn leave<C: Crew>(world: &mut World, job: Entity, member: Entity) -> Departure {
    let id = entity_def::simulation_id(world, member);
    let mut job_mut = world.entity_mut(job);

    let Some(mut crew) = job_mut.get_mut::<C>() else {
        return Departure::JobGone;
    };
    let members = crew
        .members_mut()
        .expect("a job that takes no crew has nobody on it to leave");
    assert!(members.remove(&id), "a member leaves only a job it joined");
    if members.is_empty() {
        Departure::LastOut
    } else {
        Departure::OthersRemain
    }
}

/// Like [`leave`], but the last worker out takes the marker with it — for a crew whose
/// component says nothing beyond "somebody is on this".
pub(super) fn leave_and_unmark<C: Crew>(world: &mut World, job: Entity, member: Entity) {
    match leave::<C>(world, job, member) {
        Departure::LastOut => {
            world.entity_mut(job).remove::<C>();
        }
        Departure::OthersRemain | Departure::JobGone => {}
    }
}

/// Whether the crew on `job` shuts `newcomer` out of it, given what `shares` says
/// about a worker's willingness to share this kind of job.
///
/// A job is exclusive when either side declines to share it, so a lone worker turns
/// every newcomer away and a stacking crew still yields to one that works alone.
/// A job that takes no crew shuts everyone out; a job no longer carrying the
/// component shuts nobody out. What being
/// shut out means for the newcomer's order — waiting in place for the crew to
/// clear, or giving the job up — is the caller's to decide.
pub(super) fn excludes<C: Crew>(
    world: &World,
    job: Entity,
    newcomer: Entity,
    shares: impl Fn(&World, Entity) -> bool,
) -> bool {
    let Some(crew) = world.entity(job).get::<C>() else {
        return false;
    };
    let Some(members) = crew.members() else {
        return true;
    };
    let newcomer_shares = shares(world, newcomer);

    members
        .iter()
        .filter_map(|&id| world.resource::<EntityIndex>().alive(id))
        .filter(|&other| other != newcomer)
        .any(|other| !newcomer_shares || !shares(world, other))
}
