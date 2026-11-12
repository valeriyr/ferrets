//! Per-tick field recompute: grows every standing source's reach, re-derives
//! what the sources sustain, lets unsustained coverage decay by each field's
//! policy, and reads how far the field of each source holding up less than
//! it reaches still lies.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};

use crate::{
    components::{
        field_source::{Emitted, FieldReach, FieldSourceState, FieldSourcesComponent, Growth},
        hidden::HiddenComponent,
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    field_cover,
    fields::FieldGrid,
    map::Map,
    session::player_id::PlayerId,
};
use ferrets_content::{
    field::{Emission, FieldDecay, FieldDef, FieldGrowth, FieldId, FieldSourceDef},
    registry::ContentRegistry,
};

/// What a source projects this recompute, its area read.
#[derive(Clone, Copy)]
enum Emitting {
    /// As much as when the entity operates.
    Full,
    /// A patch this many cells out from the footprint.
    Held(u32),
    /// Nothing.
    Nothing,
}

/// Whether a source's body is still rising.
#[derive(Clone, Copy)]
enum Stage {
    /// Still rising into the world.
    Rising,
    /// Risen, or arrived finished.
    Risen,
}

/// One source's areas read for this recompute.
#[derive(Clone, Copy)]
struct SourceRead {
    /// Whether its body is still rising.
    stage: Stage,
    /// What it projects.
    emitting: Emitting,
    /// Its area in cells.
    area: u32,
    /// How far its field reaches before it grows: a start read no farther
    /// than the area, or the reach it holds, read again off the grid when it
    /// held up less than it at the last recompute and its owner is in the
    /// field's `lost_by`.
    grown: u32,
}

/// A source holding up less than it reaches this recompute.
struct Reduced {
    /// The entity it belongs to.
    entity: Entity,
    /// Its index among the entity's sources.
    index: usize,
    /// The player whose cover it reads.
    player: PlayerId,
    /// The footprint its field lies around.
    footprint: CellRect,
    /// What it projects this recompute.
    holds: Emitted,
}

/// One source's projection this tick.
struct Patch {
    /// The field projected.
    field: FieldId,
    /// The player whose cover it is.
    player: PlayerId,
    /// The footprint it spreads from.
    footprint: CellRect,
    /// How far from the footprint it reaches, in cells.
    reach: u32,
}

/// Recomputes every field for the current tick.
///
/// Sources are read from the alive index, so a dying source stops projecting
/// the tick it dies. While rising a source projects its `while_rising`
/// emission and disabled its `while_disabled` one — one of: all it would
/// project operating, a held patch, or nothing. A field outage is read from
/// the grid as it stands before this pass rewrites it, so a source switched
/// off by losing a field follows one tick after the field does; a buff
/// outage, or the rise, is read as it is. A source reads its start at its
/// first recompute once its body has risen, or while rising projecting all
/// it would operating, and takes as its reach what it places then, never
/// below a patch it held while rising.
/// Operating, a gradual source grows one cell per cycle up to its area.
/// Sustained cells are re-derived from scratch, covered cells absorb them,
/// and the rest decays per field: at once, from the edge inward every cycle,
/// or never. A source holding up less than it reaches — held, emitting
/// nothing, or over a lowered area — keeps as its reach only how far its
/// field still lies around it, never below a patch it holds: read off the
/// grid before it grows and again as this recompute leaves the grid,
/// whenever its owner is in the field's `lost_by`.
///
/// Every fold is either commutative or reads a snapshot taken before the pass
/// writes, so source order cannot reach the grid.
pub fn recompute_fields(world: &mut World) {
    let fields: Vec<(FieldId, FieldDef)> = {
        let registry = world.resource::<ContentRegistry>();
        registry
            .field_ids()
            .map(|id| (id, *registry.field_def(id)))
            .collect()
    };
    let (patches, reduced) = advance_sources(world, &fields);
    sustain_and_decay(world, &fields, &patches);
    read_reduced(world, &fields, &reduced);
}

/// Advances every standing source's growth and lists what each projects and
/// which hold up less than they reach.
fn advance_sources(
    world: &mut World,
    fields: &[(FieldId, FieldDef)],
) -> (Vec<Patch>, Vec<Reduced>) {
    let mut patches: Vec<Patch> = Vec::new();
    let mut reduced: Vec<Reduced> = Vec::new();
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if world.entity(entity).contains::<HiddenComponent>() {
            continue;
        }
        let def = entity_def::of(world, entity);
        if def.field_sources.is_empty() {
            continue;
        }
        let Some(player) = entity_def::owner(world, entity) else {
            continue;
        };
        let footprint = entity_def::location(world, entity).footprint();
        let sources = def.field_sources.clone();
        let reads = read_sources(world, entity, fields, &sources, player, footprint);

        let mut component = world
            .get_mut::<FieldSourcesComponent>(entity)
            .expect("a type with field sources carries their state");
        for (index, ((source, read), state)) in sources
            .iter()
            .zip(reads)
            .zip(component.0.iter_mut())
            .enumerate()
        {
            let (projected, growth) = advance(source, *state, read);
            state.growth = growth;
            // A source holding up less than it reaches reads, once the grid
            // has changed, how far its field still lies.
            if let Some(reach) = growth.reach() {
                let holds_less = match projected {
                    Emitted::Reach(holds) => holds < reach,
                    Emitted::Nothing => true,
                };
                if holds_less {
                    reduced.push(Reduced {
                        entity,
                        index,
                        player,
                        footprint,
                        holds: projected,
                    });
                }
            }
            match projected {
                Emitted::Reach(reach) => patches.push(Patch {
                    field: source.field(),
                    player,
                    footprint,
                    reach,
                }),
                Emitted::Nothing => {}
            }
            state.last_emitted = projected;
        }
    }
    (patches, reduced)
}

/// Reads each of `entity`'s `sources` for this recompute: what it projects,
/// its area, and how far its field reaches before it grows. An area read
/// from a stat is read as the entity holds it now.
fn read_sources(
    world: &World,
    entity: Entity,
    fields: &[(FieldId, FieldDef)],
    sources: &[FieldSourceDef],
    player: PlayerId,
    footprint: CellRect,
) -> Vec<SourceRead> {
    let operation = entity_def::operation(world, entity);
    let stage = match operation {
        Operation::Rising => Stage::Rising,
        Operation::Operating | Operation::Disabled(_) | Operation::Stranded => Stage::Risen,
    };
    let area_reach = |area| entity_def::area_reach(world, entity, area);
    let states = world
        .get::<FieldSourcesComponent>(entity)
        .expect("a type with field sources carries their state");
    sources
        .iter()
        .zip(&states.0)
        .map(|(source, state)| {
            let emission = match operation {
                Operation::Operating => Emission::Full,
                Operation::Rising => source.while_rising(),
                Operation::Disabled(_) => source.while_disabled(),
                // Off the map, a source stands on no cell to emit from.
                Operation::Stranded => unreachable!("a stranded body is hidden"),
            };
            let emitting = match emission {
                Emission::Full => Emitting::Full,
                Emission::Held(area) => Emitting::Held(area_reach(area)),
                Emission::Nothing => Emitting::Nothing,
            };
            let area = area_reach(source.area());
            let grown = match state.growth {
                // A start is never below the patch held while rising.
                Growth::Unread(FieldReach::Initial) => match source.growth() {
                    FieldGrowth::Gradual { starts_at, .. } => {
                        let start = area_reach(starts_at);
                        let start = match state.last_emitted {
                            Emitted::Reach(held) => start.max(held),
                            Emitted::Nothing => start,
                        };
                        start.min(area)
                    }
                    FieldGrowth::Instant => area,
                },
                Growth::Unread(FieldReach::Full) => area,
                Growth::Spanning { reach } | Growth::Growing { reach, .. } => {
                    match state.last_emitted {
                        // Holding up its whole reach, it sustained every cell
                        // of it.
                        Emitted::Reach(held) if held == reach => reach,
                        Emitted::Reach(_) | Emitted::Nothing => reach_still_lying(
                            world,
                            fields,
                            source.field(),
                            footprint,
                            player,
                            reach,
                        ),
                    }
                }
            };
            SourceRead {
                stage,
                emitting,
                area,
                grown,
            }
        })
        .collect()
}

/// Where the growth of `state`, a source of `source`, goes this recompute
/// given its `read`: what it projects and its growth after. A source read
/// once risen takes the wider of what it places and the patch it held while
/// rising.
fn advance(
    source: &FieldSourceDef,
    state: FieldSourceState,
    read: SourceRead,
) -> (Emitted, Growth) {
    let growth = state.growth;
    let SourceRead {
        stage,
        emitting,
        area,
        grown,
    } = read;
    match (growth, emitting) {
        (Growth::Unread(_), Emitting::Nothing) => match stage {
            Stage::Rising => (Emitted::Nothing, growth),
            Stage::Risen => {
                let lies = match state.last_emitted {
                    Emitted::Reach(patch) => patch.min(area),
                    Emitted::Nothing => 0,
                };
                (Emitted::Nothing, read_at(source, lies))
            }
        },
        (Growth::Spanning { .. } | Growth::Growing { .. }, Emitting::Nothing) => {
            (Emitted::Nothing, growth.with_reach(grown))
        }
        (Growth::Unread(_), Emitting::Held(held)) => {
            let held = held.min(area);
            match stage {
                Stage::Rising => (Emitted::Reach(held), growth),
                Stage::Risen => {
                    let lies = match state.last_emitted {
                        Emitted::Reach(patch) => held.max(patch.min(area)),
                        Emitted::Nothing => held,
                    };
                    (Emitted::Reach(held), read_at(source, lies))
                }
            }
        }
        (Growth::Spanning { .. } | Growth::Growing { .. }, Emitting::Held(held)) => {
            let held = held.min(area);
            (Emitted::Reach(held), growth.with_reach(grown.max(held)))
        }
        (Growth::Unread(_) | Growth::Spanning { .. } | Growth::Growing { .. }, Emitting::Full) => {
            match source.growth() {
                FieldGrowth::Instant => (
                    Emitted::Reach(area),
                    Growth::Spanning {
                        reach: area.max(grown),
                    },
                ),
                FieldGrowth::Gradual { cycle, .. } => {
                    let mut countdown = match growth {
                        Growth::Unread(_) => cycle,
                        Growth::Growing { countdown, .. } => countdown,
                        Growth::Spanning { .. } => {
                            unreachable!("a gradual source grows by cycles")
                        }
                    };
                    let mut projected = grown.min(area);
                    if projected < area {
                        countdown -= 1;
                        if countdown == 0 {
                            projected += 1;
                            countdown = cycle;
                        }
                    }
                    (
                        Emitted::Reach(projected),
                        Growth::Growing {
                            reach: projected.max(grown),
                            countdown,
                        },
                    )
                }
            }
        }
    }
}

/// Re-derives what every patch sustains, lets every field's unsustained
/// cover decay by its policy.
fn sustain_and_decay(world: &mut World, fields: &[(FieldId, FieldDef)], patches: &[Patch]) {
    // Cells are derived against the map first; the grid is written after, so
    // the two resources are never borrowed together.
    let map = world.resource::<Map>();
    let sustained: Vec<(FieldId, PlayerId, Vec<CellPos>)> = patches
        .iter()
        .map(|patch| {
            let def = &fields[patch.field.index()].1;
            (
                patch.field,
                patch.player,
                field_cover::flood(map, def, patch.footprint, patch.reach),
            )
        })
        .collect();

    let mut grid = world.resource_mut::<FieldGrid>();
    for &(field, def) in fields {
        grid.reset_sustained(field);
        for (_, player, cells) in sustained.iter().filter(|(f, _, _)| *f == field) {
            for &cell in cells {
                grid.sustain(field, cell, *player);
            }
        }
        grid.grow(field);
        match def.decay() {
            FieldDecay::Instant => grid.clear_all_unsustained(field),
            FieldDecay::Never => {}
            FieldDecay::Gradual { cycle } => {
                if grid.decay_due(field, cycle) {
                    grid.recede(field);
                }
            }
        }
    }
}

/// Reads how far each reduced source's field still lies, off the grid as this
/// recompute left it, never below a patch it holds; then empties every
/// field's `lost_by`.
fn read_reduced(world: &mut World, fields: &[(FieldId, FieldDef)], reduced: &[Reduced]) {
    let lying: Vec<(Entity, usize, u32)> = reduced
        .iter()
        .map(|reduced| {
            let state = world
                .get::<FieldSourcesComponent>(reduced.entity)
                .expect("a reduced source carries its state")
                .0[reduced.index];
            let reach = state
                .growth
                .reach()
                .expect("a reduced source has read its reach");
            let lies = reach_still_lying(
                world,
                fields,
                state.field,
                reduced.footprint,
                reduced.player,
                reach,
            );
            let lies = match reduced.holds {
                Emitted::Reach(holds) => lies.max(holds),
                Emitted::Nothing => lies,
            };
            (reduced.entity, reduced.index, lies)
        })
        .collect();
    for (entity, index, lies) in lying {
        let mut component = world
            .get_mut::<FieldSourcesComponent>(entity)
            .expect("a reduced source carries its state");
        let state = &mut component.0[index];
        state.growth = state.growth.with_reach(lies);
    }
    let mut grid = world.resource_mut::<FieldGrid>();
    for &(field, _) in fields {
        grid.clear_lost_by(field);
    }
}

/// The growth a start read at `reach` takes: spanning for a source that grows
/// at once, growing with a fresh cycle for one that grows by cycles.
fn read_at(source: &FieldSourceDef, reach: u32) -> Growth {
    match source.growth() {
        FieldGrowth::Instant => Growth::Spanning { reach },
        FieldGrowth::Gradual { cycle, .. } => Growth::Growing {
            reach,
            countdown: cycle,
        },
    }
}

/// How far `field` still lies around `footprint` for `player`, out to
/// `reach`: `reach` unless the player is in the field's `lost_by`, else read
/// off the grid ([`lying_reach`]).
fn reach_still_lying(
    world: &World,
    fields: &[(FieldId, FieldDef)],
    field: FieldId,
    footprint: CellRect,
    player: PlayerId,
    reach: u32,
) -> u32 {
    let grid = world.resource::<FieldGrid>();
    if !grid.lost_by(field).contains(player) {
        return reach;
    }
    let def = &fields[field.index()].1;
    lying_reach(
        world.resource::<Map>(),
        grid,
        (field, def),
        footprint,
        player,
        reach,
    )
}

/// How far `field` still lies around `footprint` for `player`: the largest
/// reach no farther than `grown` whose every cell is in the player's cover.
/// The search starts at the farthest reach that still adds a map cell and
/// halves the reaches between.
fn lying_reach(
    map: &Map,
    grid: &FieldGrid,
    (field, def): (FieldId, &FieldDef),
    footprint: CellRect,
    player: PlayerId,
    grown: u32,
) -> u32 {
    let whole = |reach| {
        field_cover::flood(map, def, footprint, reach)
            .into_iter()
            .all(|cell| grid.covered(field, cell).contains(player))
    };
    let across = |origin: u32, size: u32, extent: u32| {
        origin.max(extent.saturating_sub(origin.saturating_add(size)))
    };
    let farthest = across(footprint.origin.x, footprint.size.width, map.width()).saturating_add(
        across(footprint.origin.y, footprint.size.height, map.height()),
    );
    let top = grown.min(farthest);
    if whole(top) {
        return grown;
    }
    // `lies` is whole or the floor, `broken` is not.
    let (mut lies, mut broken) = (0, top);
    while broken - lies > 1 {
        let middle = lies + (broken - lies) / 2;
        if whole(middle) {
            lies = middle;
        } else {
            broken = middle;
        }
    }
    lies
}
