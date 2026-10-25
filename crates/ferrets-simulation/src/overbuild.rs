//! Resource sources covered by what is raised over them, and given back.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{registry::ContentRegistry, resource::DepletionPolicy};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_physics::body;

use crate::{
    components::{build::OverbuiltComponent, hidden::Inside, resource::ResourceSourceComponent},
    entity_def,
    entity_index::EntityIndex,
    events::DeathCause,
    occupancy::GridMark,
    spawn::{self, FieldReach},
};

/// What a footprint covers as it is set down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cover {
    /// Nothing: the footprint takes open ground.
    Nothing,
    /// A resource source of the type the footprint is raised over, standing
    /// exactly under it.
    Source(Entity),
}

/// The grid marks to lift and the bodies to ignore when judging the ground
/// for a footprint set down on `cover`: the source's own, as though it had
/// already come off the map; none on open ground.
pub(crate) fn set_aside(world: &World, cover: Cover) -> (Vec<GridMark>, Vec<Entity>) {
    match cover {
        Cover::Nothing => (Vec::new(), Vec::new()),
        Cover::Source(source) => (
            entity_def::grid_mark(world, source).into_iter().collect(),
            vec![source],
        ),
    }
}

/// Takes the source `cover` names off the map, its footprint off the grid, so
/// what is set down over it takes its cells, as [`set_aside`] judged it;
/// nothing on open ground.
pub(crate) fn lift(world: &mut World, cover: Cover) {
    match cover {
        Cover::Source(source) => {
            spawn::lift_footprint(world, source);
            spawn::mark_hidden(world, source, Inside::Nothing);
        }
        Cover::Nothing => {}
    }
}

/// Settles the source [`lift`] took off the map once the entity set down over
/// it is spawned: covered by `spawned`. Nothing on open ground, spawned or
/// not.
///
/// Panics for a source with nothing spawned over it: the ground over a source
/// is judged with the source set aside (see [`set_aside`]) before it is
/// lifted.
pub(crate) fn settle(world: &mut World, cover: Cover, spawned: Option<Entity>) {
    match (cover, spawned) {
        (Cover::Source(source), Some(over)) => cover_source(world, source, over),
        (Cover::Source(_), None) => {
            unreachable!("the ground over a covered source was judged to take what covers it")
        }
        (Cover::Nothing, Some(_) | None) => {}
    }
}

/// The resource source of type `over` standing exactly on `footprint`, if one
/// does: the lowest id among several.
pub(crate) fn source_under(world: &World, over: &str, footprint: CellRect) -> Option<Entity> {
    source_where(world, over, |standing| standing == footprint)
}

/// The resource source of type `over` standing over `cell`, if one does: the
/// lowest id among several.
pub(crate) fn source_covering(world: &World, over: &str, cell: CellPos) -> Option<Entity> {
    source_where(world, over, |standing| standing.contains(cell))
}

/// Puts back the resource source `entity` was raised over, on the cells it was
/// covered on and with what it had left, announcing the return; nothing for an
/// entity raised on open ground. One drained dry comes back empty or not at
/// all, as the source type's own depletion says.
///
/// A caller uncovers once the entity's own footprint is off the grid. When
/// those cells are no longer free even so — something walked onto ground the
/// cover never claimed — the source does not come back at all, the way blocked
/// ground leaves no rubble.
///
/// A cover that wears a form which is no resource source holds nothing, so what
/// it gives back is an empty source or none at all, as if it had been drained.
pub(crate) fn uncover_source(world: &mut World, entity: Entity) {
    let Some(overbuilt) = world.entity(entity).get::<OverbuiltComponent>().copied() else {
        return;
    };
    let amount = world
        .entity(entity)
        .get::<ResourceSourceComponent>()
        .map_or(0, |source| source.amount);
    let (type_name, depletion) = {
        let def = world.resource::<ContentRegistry>().def(overbuilt.uncovers);
        (
            def.name.clone(),
            def.resource_source
                .as_ref()
                .expect("an overbuilt type is a resource source")
                .depletion(),
        )
    };
    if amount == 0 {
        match depletion {
            DepletionPolicy::Persist => {}
            DepletionPolicy::Destroy => return,
        }
    }
    let by = entity_def::simulation_id(world, entity);
    if let Some((source, _)) = spawn::spawn_uncovered(
        world,
        &type_name,
        FixedUVec2::from(overbuilt.anchor),
        None,
        FieldReach::Full,
        by,
    ) {
        world
            .entity_mut(source)
            .get_mut::<ResourceSourceComponent>()
            .expect("an overbuilt type is a resource source")
            .amount = amount;
    }
}

/// Ends `source`, which [`lift`] took off the map, under the entity `over`
/// raised over it, handing it what the source had left: the source's
/// footprint is already `over`'s, so it leaves without freeing anything, and
/// `over` remembers what to put back. The source dies as overbuilt.
/// [`uncover_source`] is the other half.
fn cover_source(world: &mut World, source: Entity, over: Entity) {
    let amount = world
        .entity(source)
        .get::<ResourceSourceComponent>()
        .expect("a source covered is a resource source")
        .amount;
    let uncovers = entity_def::type_id(world, source);
    let anchor = body::anchor(entity_def::whereabouts(world, source));
    spawn::despawn_entity(world, source, DeathCause::Overbuilt);

    let mut over_mut = world.entity_mut(over);
    over_mut
        .get_mut::<ResourceSourceComponent>()
        .expect("a type raised over a source is a resource source")
        .amount = amount;
    over_mut.insert(OverbuiltComponent { uncovers, anchor });
}

/// The resource source of type `over` on the map whose footprint `holds`
/// takes, if one does: the lowest id among several.
fn source_where(world: &World, over: &str, holds: impl Fn(CellRect) -> bool) -> Option<Entity> {
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .find(|&entity| {
            world.entity(entity).contains::<ResourceSourceComponent>()
                && entity_def::of(world, entity).name == over
                && entity_def::stands_on_grid(world, entity)
                && holds(entity_def::footprint_rect(world, entity))
        })
}
