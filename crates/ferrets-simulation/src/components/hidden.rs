//! Marker for entities that are temporarily off the map.

use bevy_ecs::prelude::*;

use ferrets_geometry::cell_rect::CellRect;

use crate::{
    beside::{Beside, Lean},
    components::location::LocationComponent,
    links::LinkKey,
    simulation_id::SimulationId,
};

/// What a hidden entity is off the map inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inside {
    /// The transporter it rides aboard.
    Transporter(SimulationId),
    /// The job it works hidden inside.
    Job(SimulationId),
    /// The link it went into by one of its gateways.
    Link(LinkKey),
    /// It waits off the map for a free cell to return to, retried every tick.
    Waiting {
        /// What the return searches beside, read again on every attempt.
        beside: Beside,
        /// Which cell beside it the return takes first.
        lean: Lean,
        /// The footprint the return last searched beside; it stands in for a
        /// container gone from the world.
        last: CellRect,
    },
    /// Inside nothing: taken off the map for no container, or let go of what
    /// held it.
    Nothing,
}

/// Marks an entity that is temporarily not present on the map.
///
/// A hidden entity has no [`LocationComponent`], does not occupy the navigation
/// grid, cannot be selected, and cannot be targeted.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct HiddenComponent {
    /// Where the entity stood when it was taken off the map, or, let go of
    /// by what held it, where that holder was.
    pub went_in: LocationComponent,
    /// What the entity is off the map inside.
    pub inside: Inside,
}

impl HiddenComponent {
    /// Creates a new `HiddenComponent` with the given data.
    #[inline]
    pub fn new(went_in: LocationComponent, inside: Inside) -> Self {
        Self { went_in, inside }
    }
}

/// Records `inside` as what the hidden `entity` is off the map inside now,
/// announcing nothing.
///
/// Panics for an entity that is not hidden.
pub(crate) fn record_inside(world: &mut World, entity: Entity, inside: Inside) {
    world
        .entity_mut(entity)
        .get_mut::<HiddenComponent>()
        .expect("only a hidden entity moves inside something else")
        .inside = inside;
}

/// Records `went_in` as where the hidden `entity` is taken to have gone off the map, announcing
/// nothing.
///
/// Panics for an entity that is not hidden.
pub(crate) fn record_went_in(world: &mut World, entity: Entity, went_in: LocationComponent) {
    world
        .entity_mut(entity)
        .get_mut::<HiddenComponent>()
        .expect("only a hidden entity went off the map somewhere")
        .went_in = went_in;
}

/// Records `searched` as the footprint the waiting `entity`'s return last
/// searched beside.
///
/// Panics for an entity that is not waiting on a return.
pub(crate) fn note_last_search(world: &mut World, entity: Entity, searched: CellRect) {
    let mut entity_mut = world.entity_mut(entity);
    let mut marker = entity_mut
        .get_mut::<HiddenComponent>()
        .expect("only a hidden entity waits on a return");
    match &mut marker.inside {
        Inside::Waiting { last, .. } => *last = searched,
        Inside::Transporter(_) | Inside::Job(_) | Inside::Link(_) | Inside::Nothing => {
            panic!("only a waiting entity's return searches")
        }
    }
}
