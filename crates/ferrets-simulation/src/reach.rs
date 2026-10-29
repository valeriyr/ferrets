//! Whether a body reaches a goal within a range: from the cells it stands on,
//! from a spot it would stand at, or, for one that walks, from somewhere in
//! the regions it walks in — and how far a body stands from a set of cells.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::registry::ContentRegistry;
use ferrets_geometry::{cell_rect::CellRect, cell_size::CellSize};
use ferrets_nav_grid::mover_shape::MoverShape;
use ferrets_physics::body;

use crate::{berths, entity_def, landing::ComesFrom, map::Map, regions};

/// Whether `entity`, standing on the cells `from`, reaches `goal` within
/// `range`: a touch needs an edge between them its domain could step. Its
/// shape is read only once `goal` lies within `range`.
pub(crate) fn reaches(
    world: &World,
    entity: Entity,
    from: CellRect,
    goal: CellRect,
    range: u32,
) -> bool {
    let map = world.resource::<Map>();
    if !map.projection().in_range_for_rects(from, goal, range) {
        return false;
    }
    let location = entity_def::of(world, entity)
        .location
        .expect("only entities with a location reach");
    let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &location);
    reaches_from(map, from, shape, goal, range)
}

/// Whether `entity` reaches `goal` within `range` from every cell it stands
/// on ([`entity_def::standing_rect`]), as [`reaches`] judges it.
///
/// Panics if `entity` is not a simulation entity, is off the map, or its type
/// declares no location.
pub(crate) fn in_reach(world: &World, entity: Entity, goal: CellRect, range: u32) -> bool {
    reaches(
        world,
        entity,
        entity_def::standing_rect(world, entity),
        goal,
        range,
    )
}

/// The distance from the cells `from` to the cells `entity` stands on
/// ([`entity_def::standing_rect`]), by the map's projection.
///
/// Panics if `entity` is not a simulation entity, is off the map, or its type
/// declares no location.
pub(crate) fn distance_to(world: &World, from: CellRect, entity: Entity) -> u32 {
    world
        .resource::<Map>()
        .projection()
        .distance_for_rects(from, entity_def::standing_rect(world, entity))
}

/// How a body gets within reach of a goal from where it comes.
pub(crate) enum Approach {
    /// From the cells it comes from: a body that never walks.
    Stands {
        /// The cells it reaches from.
        rect: CellRect,
        /// Its shape.
        shape: MoverShape,
    },
    /// By walking to an anchor in one of `regions` of `shape`.
    Walks {
        /// Its shape.
        shape: MoverShape,
        /// The regions of its shape it walks into from where it comes.
        regions: Vec<u32>,
    },
}

impl Approach {
    /// How `entity`, on the map, gets within reach of a goal: one seated in a
    /// berth as it would coming out of that berth; otherwise a mover by
    /// walking in the region of its shape it stands in, a body that never
    /// walks from the cells it stands on.
    ///
    /// Panics if `entity` is not a simulation entity, is off the map, or its
    /// type declares no location.
    pub(crate) fn from_where_it_is(world: &World, entity: Entity) -> Self {
        if entity_def::attached(world, entity) {
            let berth = berths::held_berth(world, entity);
            return Approach::coming_from(
                world,
                entity,
                ComesFrom::steps_out_of(world, entity, berth),
            );
        }
        let def = entity_def::of(world, entity);
        let location = def
            .location
            .expect("only a body that stands somewhere approaches");
        let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &location);
        if def.can_move() {
            let anchor = body::anchor(entity_def::position(world, entity));
            Approach::Walks {
                shape,
                regions: regions::walked_into(
                    world.resource::<Map>(),
                    shape,
                    ComesFrom::Point(anchor),
                ),
            }
        } else {
            Approach::Stands {
                rect: entity_def::standing_rect(world, entity),
                shape,
            }
        }
    }

    /// How `entity`, off the map and set down out of `from` when it comes
    /// back, gets within reach of a goal: a mover by walking in the regions
    /// it would walk into from there, a body that never walks from the cells
    /// it would come out of.
    ///
    /// Panics if `entity` is not a simulation entity, or its type declares no
    /// location.
    pub(crate) fn coming_from(world: &World, entity: Entity, from: ComesFrom) -> Self {
        let def = entity_def::of(world, entity);
        let location = def
            .location
            .expect("only a body that stands somewhere approaches");
        let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &location);
        if def.can_move() {
            Approach::Walks {
                shape,
                regions: regions::walked_into(world.resource::<Map>(), shape, from),
            }
        } else {
            match from {
                ComesFrom::Footprint { rect, .. } => Approach::Stands { rect, shape },
                ComesFrom::Point(cell) => Approach::Stands {
                    rect: CellRect::new(cell, CellSize::ONE),
                    shape,
                },
            }
        }
    }

    /// Whether a body approaching so gets within `range` of the cells
    /// `target` stands on: from the cells it stands on, or at an anchor in a
    /// region it walks into that accepts them — a touch only across an edge it
    /// could step.
    ///
    /// Panics if `target` is not a simulation entity, is off the map, or its
    /// type declares no location.
    pub(crate) fn reaches(&self, world: &World, target: Entity, range: u32) -> bool {
        let goal = entity_def::standing_rect(world, target);
        let map = world.resource::<Map>();
        match self {
            Approach::Stands { rect, shape } => reaches_from(map, *rect, *shape, goal, range),
            Approach::Walks { shape, regions } => {
                let grid = map.nav_grid();
                regions::anchors_within(grid, goal, shape.size, range).any(|anchor| {
                    grid.accepts(map.projection(), anchor, *shape, goal, range)
                        && regions::holds(map, *shape, regions, anchor)
                })
            }
        }
    }
}

/// Whether a body of `shape` standing on the cells `from` reaches `goal`
/// within `range`: a touch needs an edge between them its domain could step.
fn reaches_from(map: &Map, from: CellRect, shape: MoverShape, goal: CellRect, range: u32) -> bool {
    map.projection().in_range_for_rects(from, goal, range)
        && map
            .nav_grid()
            .reaches(map.projection(), from, shape, goal, range)
}
