//! Where a body set down away from where it stood may land: ground that takes
//! its footprint — over a source of its type, for a type raised over one —
//! admitted by its type's placement rules and the field rules it is held to,
//! seen as the landing word asks by the team that sets it down, and reached
//! from where it comes: for a mover, ground it could walk back to there; for
//! a body that never walks, a level it steps onto from there, or any level
//! coming down from above. A mover's walk goal is found the same way, but for
//! its type's placement rules.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_type_def::EntityTypeDef, field::FieldPlacement, landing::Landing,
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_nav_grid::{layer_id::LayerId, mover_shape::MoverShape, nav_grid::NavGrid};

use crate::{
    beside::Beside,
    components::location::LocationComponent,
    entity_def,
    entity_index::EntityIndex,
    fields,
    ground::{self, GroundHolder},
    map::Map,
    occupancy::{GridMark, OccupancyClass},
    overbuild::{self, Cover},
    session::{GameSession, ai_vision::AiVision, player_id::PlayerId, player_slot::PlayerSlot},
    spawn,
    visibility::{CellVisibility, VisibilityGrid},
};

/// How many rings out from a rectangle a set-down searches when nothing
/// bounds it closer.
const MAX_RADIUS: u32 = 8;

/// Where a body set down comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ComesFrom {
    /// Out of a footprint standing on the map.
    Footprint {
        /// The cells it covers.
        rect: CellRect,
        /// The layer it stands on.
        layer: LayerId,
        /// How many cells out of it the body comes.
        reach: u32,
    },
    /// At a cell of the map.
    Point(CellPos),
}

impl ComesFrom {
    /// Out of `entity`'s footprint, a cell out, on the layer it stands on.
    ///
    /// Panics if `entity` is not a simulation entity, or its type declares no
    /// location.
    pub(crate) fn of(world: &World, entity: Entity) -> Self {
        ComesFrom::within(world, entity, 1)
    }

    /// Out of `entity`'s footprint, `reach` cells out, on the layer it stands
    /// on.
    ///
    /// Panics if `entity` is not a simulation entity, or its type declares no
    /// location.
    pub(crate) fn within(world: &World, entity: Entity, reach: u32) -> Self {
        ComesFrom::Footprint {
            rect: entity_def::footprint_rect(world, entity),
            layer: layer_of(world, entity),
            reach,
        }
    }

    /// Out of where the seated `entity` sits, a cell out: its job's footprint
    /// when `berth`, the cell it sits at, lies inside it, else `berth` alone,
    /// on the job's layer, or on `entity`'s own once the job is gone.
    ///
    /// Panics if `entity` sits in no berth, or a type it or its job wears
    /// declares no location.
    pub(crate) fn berth(world: &World, entity: Entity, berth: CellRect) -> Self {
        let (rect, layer) = match entity_def::berth_job(world, entity) {
            Some(job) => {
                let footprint = entity_def::footprint_rect(world, job);
                let rect = if footprint.contains(berth.origin) {
                    footprint
                } else {
                    berth
                };
                (rect, layer_of(world, job))
            }
            None => (berth, layer_of(world, entity)),
        };
        ComesFrom::Footprint {
            rect,
            layer,
            reach: 1,
        }
    }

    /// Out of the footprint `entity` is set down beside, as
    /// [`entity_def::beside_rect`] gives it, `reach` cells out, on the layer
    /// of the container standing there — the first that stands of what
    /// `beside` names and what that is inside — or `entity`'s own once none
    /// remains.
    ///
    /// Panics if `entity` is not a simulation entity, or its type declares no
    /// location.
    pub(crate) fn beside(world: &World, entity: Entity, beside: Beside, reach: u32) -> Self {
        let standing = match beside {
            Beside::Container(id) => world
                .resource::<EntityIndex>()
                .any(id)
                .map(|container| standing_container(world, container)),
            Beside::WentIn | Beside::Footprint(_) => None,
        };
        ComesFrom::Footprint {
            rect: entity_def::beside_rect(world, entity, beside),
            layer: layer_of(world, standing.unwrap_or(entity)),
            reach,
        }
    }
}

/// The ground holders for bodies set down together.
enum Holders {
    /// Not read yet: no body on the static plane has landed.
    Unread,
    /// Read when the first did.
    Read(Vec<GroundHolder>),
}

/// Bodies set down together, each on ground none of the others was given.
pub(crate) struct Landings<'a> {
    /// The player whose team must see the cells.
    player: PlayerId,
    /// Which cells the team must see.
    landing: Landing,
    /// The field rules every footprint must meet for `player`, beside each
    /// type's own placement rules.
    within: &'a [FieldPlacement],
    /// What the bodies being moved mark on the grid now: those cells do not
    /// stand in the way of one another.
    lifted: &'a [GridMark],
    /// The bodies being moved, those that mark no cells included: none of
    /// them holds ground against another.
    ignored: &'a [Entity],
    /// The ground holders, none of `ignored` counted, once a body on the
    /// static plane first lands.
    holders: Holders,
    /// The footprints given out so far.
    taken: Vec<CellRect>,
}

impl<'a> Landings<'a> {
    /// Bodies set down for `player` as `landing` says, inside the `within`
    /// field rules, the ones being moved marking `lifted` and being
    /// `ignored`; nothing given out yet.
    pub(crate) fn new(
        player: PlayerId,
        landing: Landing,
        within: &'a [FieldPlacement],
        lifted: &'a [GridMark],
        ignored: &'a [Entity],
    ) -> Self {
        Self {
            player,
            landing,
            within,
            lifted,
            ignored,
            holders: Holders::Unread,
            taken: Vec::new(),
        }
    }
}

/// Whether `player` sees every cell of `footprint`, all on the map, as
/// `landing` asks — see [`sees_in`].
pub(crate) fn sees(world: &World, player: PlayerId, landing: Landing, footprint: CellRect) -> bool {
    sees_in(
        world.resource::<VisibilityGrid>(),
        world.resource::<GameSession>(),
        player,
        landing,
        footprint,
    )
}

/// Whether `player` sees every cell of `footprint`, all on the map, as
/// `landing` asks, by the vision its seat in `session` declares: an
/// omniscient seat sees every cell, a filtered one what its team's fog shows
/// on `grid`, a free seat none.
pub fn sees_in(
    grid: &VisibilityGrid,
    session: &GameSession,
    player: PlayerId,
    landing: Landing,
    footprint: CellRect,
) -> bool {
    let vision = session
        .slot(player)
        .and_then(PlayerSlot::senses)
        .map(|(vision, _)| vision);
    let seen = |enough: fn(CellVisibility) -> bool| match vision {
        Some(AiVision::Omniscient) => true,
        Some(AiVision::Filtered) => footprint
            .cells()
            .all(|cell| enough(grid.visibility_to(session, player, cell.x, cell.y))),
        None => false,
    };
    match landing {
        Landing::Anywhere => true,
        Landing::Explored => seen(|cell| match cell {
            CellVisibility::Explored | CellVisibility::Visible => true,
            CellVisibility::Unexplored => false,
        }),
        Landing::Sighted => seen(|cell| match cell {
            CellVisibility::Visible => true,
            CellVisibility::Unexplored | CellVisibility::Explored => false,
        }),
    }
}

/// How far from where it starts a body may be set down.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Bound {
    /// Anchored on this cell alone.
    At(CellPos),
    /// Anywhere within the placement search radius.
    Radius,
    /// Within `range` cells of `rect`.
    Range {
        /// The footprint the range is measured from.
        rect: CellRect,
        /// How many cells from it.
        range: u32,
    },
}

/// One body to set down, and where to look for its cell.
#[derive(Clone, Copy)]
pub(crate) struct Lander<'a> {
    /// What it is.
    pub(crate) def: &'a EntityTypeDef,
    /// Whose it is, if anyone's.
    pub(crate) owner: Option<PlayerId>,
    /// Where it comes from.
    pub(crate) from: ComesFrom,
    /// How far it may be set down.
    pub(crate) bound: Bound,
    /// The rectangle the search rings grow around.
    pub(crate) around: CellRect,
    /// The rectangle whose nearest cells each ring tries first.
    pub(crate) toward: CellRect,
}

/// The anchor `lander` lands on among `landings`, as [`set_down`] finds it,
/// over what the bodies moved together hold, clear of every footprint given
/// out, inside the field rules of `landings` and seen as `landings` asks. Its
/// footprint is then given out. `None` when nothing within its bound will do,
/// and nothing is given out.
///
/// Panics for a type that stands nowhere.
pub(crate) fn land(world: &World, landings: &mut Landings, lander: Lander) -> Option<CellPos> {
    let class = OccupancyClass::of(lander.def);
    // The ground holders are read once for every body set down together, and
    // lent back to `landings` after this one.
    let holders: Vec<GroundHolder> = match class {
        OccupancyClass::Static => match std::mem::replace(&mut landings.holders, Holders::Unread) {
            Holders::Read(holders) => holders,
            Holders::Unread => ground::ground_holders(world, landings.ignored),
        },
        OccupancyClass::Claim => Vec::new(),
    };
    let lifted = landings.lifted;
    let cell = land_on(
        world,
        landings,
        lander,
        Ground::Open {
            lifted,
            holders: &holders,
        },
    );
    match class {
        OccupancyClass::Static => landings.holders = Holders::Read(holders),
        OccupancyClass::Claim => {}
    }
    cell
}

/// [`land`] for a type raised over a resource source of type `over`: a cell
/// whose footprint a source of that type stands exactly on, in place of open
/// ground, and whose ground takes the footprint once the source is off it;
/// with the source found there.
///
/// Panics for a type that stands nowhere.
pub(crate) fn land_over(
    world: &World,
    landings: &mut Landings,
    over: &str,
    lander: Lander,
) -> Option<(CellPos, Entity)> {
    let (lifted, ignored) = (landings.lifted, landings.ignored);
    let cell = land_on(
        world,
        landings,
        lander,
        Ground::Over {
            over,
            lifted,
            ignored,
        },
    )?;
    let size = lander.def.location.expect("a landed type stands").size();
    let source = overbuild::source_under(world, over, CellRect::new(cell, size))
        .expect("a cell is landed over only where a source stands");
    Some((cell, source))
}

/// The anchor `lander` lands on, ring by ring outward from `around` and
/// within each ring the cells nearest `toward` first: ground that takes its
/// footprint over what `lifted` marks, kept off by no ground holder but
/// `ignored`, within its bound, admitted by its type's placement rules and
/// `also`, and reached from where it comes. `None` when nothing within its
/// bound will do.
///
/// Out of a footprint on its own layer or a layer below, a body lands only at
/// a level it steps onto across the footprint's edge, and a mover in a region
/// of its shape it steps into within the footprint's reach — across an edge
/// it could step for a reach of one, by distance alone beyond. Coming down
/// from a layer above, it lands at any level, the highest under the footprint
/// first: a mover in any region within that reach, a body that never walks
/// anywhere its bound and ground allow. Set down at a
/// point, a mover lands in the region of the point's cell, or, where its
/// shape cannot stand on that cell, in a region it steps into from there; a
/// body that never walks at a level it steps onto from that cell. Ground over
/// the footprint or cell it comes from always counts as reached.
///
/// Panics for a type that stands nowhere.
pub(crate) fn set_down(
    world: &World,
    lander: Lander,
    lifted: &[GridMark],
    ignored: &[Entity],
    also: impl Fn(CellPos, CellRect) -> bool,
) -> Option<CellPos> {
    let holders = match OccupancyClass::of(lander.def) {
        OccupancyClass::Static => ground::ground_holders(world, ignored),
        OccupancyClass::Claim => Vec::new(),
    };
    search(
        world,
        lander,
        Ground::Open {
            lifted,
            holders: &holders,
        },
        Coming::Appears,
        also,
    )
}

/// The anchor nearest `lander.around` a mover walks to, as [`set_down`]
/// finds one but for its type's placement rules, which a walk is not held
/// to: ground that takes its footprint over what `lifted` marks, within its
/// bound, reached from where it comes, and admitted by `also`.
///
/// Panics for a type that stands nowhere.
pub(crate) fn walk_goal(
    world: &World,
    lander: Lander,
    lifted: &[GridMark],
    also: impl Fn(CellPos, CellRect) -> bool,
) -> Option<CellPos> {
    search(
        world,
        lander,
        Ground::Open {
            lifted,
            holders: &[],
        },
        Coming::Walks,
        also,
    )
}

/// How a body comes onto the cell a search finds it.
#[derive(Debug, Clone, Copy)]
enum Coming {
    /// Appearing on the map there: held to its type's placement rules.
    Appears,
    /// Walking there: held to no placement rule.
    Walks,
}

/// What ground a body set down needs.
enum Ground<'a> {
    /// Open ground over what `lifted` marks, kept off by none of `holders`.
    Open {
        /// What the bodies being moved mark on the grid now.
        lifted: &'a [GridMark],
        /// The ground holders.
        holders: &'a [GroundHolder],
    },
    /// A resource source of type `over`, its ground taking the footprint once
    /// the source is off it.
    Over {
        /// The source's type.
        over: &'a str,
        /// What the bodies being moved mark on the grid now.
        lifted: &'a [GridMark],
        /// The bodies being moved.
        ignored: &'a [Entity],
    },
}

/// The anchor for `lander` among `landings`, as [`search`] finds it on
/// `ground`, clear of every footprint given out, inside the field rules of
/// `landings` and seen as `landings` asks; its footprint is then given out.
fn land_on(
    world: &World,
    landings: &mut Landings,
    lander: Lander,
    ground: Ground,
) -> Option<CellPos> {
    fields::debug_assert_spreads_on(
        world.resource::<ContentRegistry>(),
        lander.def,
        landings.within.iter().map(FieldPlacement::field),
    );
    let taken = &landings.taken;
    let cell = search(world, lander, ground, Coming::Appears, |_, footprint| {
        !taken.iter().any(|taken| taken.intersects(footprint))
            && sees(world, landings.player, landings.landing, footprint)
            && fields::rules_allow(world, Some(landings.player), landings.within, footprint)
    })?;
    let size = lander.def.location.expect("a landed type stands").size();
    landings.taken.push(CellRect::new(cell, size));
    Some(cell)
}

/// The anchor nearest `lander.around` in [`set_down`]'s order whose footprint
/// `ground` takes, within the lander's bound, admitted by `also` and — for a
/// body `coming` onto it by appearing — by its type's placement rules, and
/// reached from where it comes as [`set_down`] says.
fn search(
    world: &World,
    lander: Lander,
    ground: Ground,
    coming: Coming,
    also: impl Fn(CellPos, CellRect) -> bool,
) -> Option<CellPos> {
    let location_def = lander
        .def
        .location
        .expect("only a type that stands somewhere lands");
    let size = location_def.size();
    let class = OccupancyClass::of(lander.def);
    let map = world.resource::<Map>();
    let projection = map.projection();
    let shape = entity_def::mover_shape(world.resource::<ContentRegistry>(), &location_def);
    let reachability = Reachability::of(map, lander.def, shape, lander.from);
    let takes = |cell: CellPos, footprint: CellRect| {
        let placed = LocationComponent::new(FixedUVec2::from(cell), spawn::DEFAULT_FACING);
        match ground {
            Ground::Open { lifted, holders } => {
                ground::can_place_clear_of(world, &placed, location_def, class, lifted, holders)
            }
            Ground::Over {
                over,
                lifted,
                ignored,
            } => match overbuild::source_under(world, over, footprint) {
                Some(source) => {
                    let (mut lifted_over, mut ignored_over) =
                        overbuild::set_aside(world, Cover::Source(source));
                    lifted_over.extend_from_slice(lifted);
                    ignored_over.extend_from_slice(ignored);
                    ground::can_place(
                        world,
                        &placed,
                        location_def,
                        class,
                        &lifted_over,
                        &ignored_over,
                    )
                }
                None => false,
            },
        }
    };
    let lands = |cell: CellPos, first: Levels| {
        let footprint = CellRect::new(cell, size);
        // The bound first; the level and reach checks never panic on a
        // footprint running off the map, and the ground refuses it before the
        // placement rules and `also` read its cells.
        let in_bound = match lander.bound {
            Bound::At(at) => cell == at,
            Bound::Radius => true,
            Bound::Range { rect, range } => projection.in_range_for_rects(footprint, rect, range),
        };
        let at_level = match first {
            Levels::Any => true,
            Levels::HighestFirst(level) => {
                highest_level(map.nav_grid(), shape.layer, footprint) == Some(level)
            }
        };
        in_bound
            && at_level
            && reachability.lands_on(map, cell)
            && takes(cell, footprint)
            && match coming {
                Coming::Appears => fields::allows_placement(world, lander.owner, lander.def, cell),
                Coming::Walks => true,
            }
            && also(cell, footprint)
    };
    // A bound content sets is the search's whole reach; the constant only
    // where nothing bounds it. No ring past the one that covers the map holds
    // a cell.
    let radius = match lander.bound {
        Bound::At(cell) => rings_to(lander.around, CellRect::new(cell, CellSize::ONE)),
        Bound::Radius => MAX_RADIUS,
        Bound::Range { rect, range } => rings_to(lander.around, rect)
            .saturating_add(range)
            .saturating_add(size.width.max(size.height) - 1),
    }
    .min(rings_to(
        lander.around,
        CellRect::new(
            CellPos::new(0, 0),
            CellSize::new(map.nav_grid().width(), map.nav_grid().height()),
        ),
    ));
    let scan = |first: Levels| {
        map.find_placement_near_toward_where(
            lander.around.origin,
            lander.around.size,
            size,
            radius,
            lander.toward,
            |cell| lands(cell, first),
        )
    };
    match first_levels(map.nav_grid(), shape.layer, lander.from) {
        Levels::Any => scan(Levels::Any),
        first @ Levels::HighestFirst(_) => scan(first).or_else(|| scan(Levels::Any)),
    }
}

/// Where a body set down may land for where it comes from.
enum Reachability {
    /// Anywhere its ground takes it: a body that never walks, coming down
    /// from a layer above.
    Unchecked,
    /// At a level a body that never walks steps onto from where it comes.
    Heights {
        /// Its shape.
        shape: MoverShape,
        /// The ground it comes from: the footprint it comes out of, or the
        /// point's cell.
        came_from: CellRect,
        /// The levels it may land at.
        levels: Vec<u32>,
    },
    /// Where a mover could walk back from.
    Regions {
        /// Its shape.
        shape: MoverShape,
        /// The ground it comes from: the footprint it comes out of, or the
        /// point's cell.
        came_from: CellRect,
        /// The regions of its shape it may land in.
        regions: Vec<u32>,
    },
}

/// Which levels a body set down tries first.
#[derive(Debug, Clone, Copy)]
enum Levels {
    /// None before the others.
    Any,
    /// Cells whose footprint's highest level is this one, before the others.
    HighestFirst(u32),
}

/// Where a footprint a body comes out of stands against the body's layer.
#[derive(Debug, Clone, Copy)]
enum Relation {
    /// On the body's own layer.
    Same,
    /// On a layer above it.
    Above,
    /// On a layer not above it.
    Below,
}

impl Reachability {
    /// Where a body of `def`, of `shape`, coming `from` there may land.
    fn of(map: &Map, def: &EntityTypeDef, shape: MoverShape, from: ComesFrom) -> Self {
        let grid = map.nav_grid();
        let hierarchy = map.hierarchy();
        let projection = map.projection();
        // The anchors within `reach` of `rect` a body steps into from there:
        // within a reach of one only across an edge it could step, as any
        // touch is judged; any, coming down from a layer above.
        let stepped_into = |rect: CellRect, reach: u32, relation: Relation| {
            anchors_within(grid, rect, shape.size, reach).filter(move |&anchor| {
                let footprint = CellRect::new(anchor, shape.size);
                match relation {
                    Relation::Above => projection.in_range_for_rects(rect, footprint, reach),
                    Relation::Same | Relation::Below => {
                        grid.reaches(projection, rect, shape, footprint, reach)
                    }
                }
            })
        };
        // The levels a body steps onto across `rect`'s edge.
        let levels_beside = |rect: CellRect, relation: Relation| {
            let mut levels: Vec<u32> = Vec::new();
            for anchor in stepped_into(rect, 1, relation) {
                let level = highest_level(grid, shape.layer, CellRect::new(anchor, shape.size))
                    .expect("an anchor within reach stands on the map");
                if !levels.contains(&level) {
                    levels.push(level);
                }
            }
            levels
        };
        let regions_of = |anchors: &mut dyn Iterator<Item = CellPos>| {
            let mut regions: Vec<u32> = Vec::new();
            for anchor in anchors {
                if let Some(region) = hierarchy.region_of(anchor, shape)
                    && !regions.contains(&region)
                {
                    regions.push(region);
                }
            }
            regions
        };
        match (def.can_move(), from) {
            (true, ComesFrom::Point(cell)) => {
                let point = CellRect::new(cell, CellSize::ONE);
                Reachability::Regions {
                    shape,
                    came_from: point,
                    // A point on ground the body refuses is left the way a
                    // footprint is.
                    regions: match hierarchy.region_of(cell, shape) {
                        Some(region) => vec![region],
                        None => regions_of(&mut stepped_into(point, 1, Relation::Same)),
                    },
                }
            }
            (true, ComesFrom::Footprint { rect, layer, reach }) => {
                let regions = match relation(grid, layer, shape.layer, rect) {
                    Relation::Above => regions_of(&mut stepped_into(rect, reach, Relation::Above)),
                    // Beyond a reach of one, distance carries the body over
                    // what lies between, but only onto the levels it steps
                    // onto across the edge.
                    relation @ (Relation::Same | Relation::Below) => {
                        let levels = levels_beside(rect, relation);
                        regions_of(&mut stepped_into(rect, reach, relation).filter(|&anchor| {
                            let level =
                                highest_level(grid, shape.layer, CellRect::new(anchor, shape.size))
                                    .expect("an anchor within reach stands on the map");
                            levels.contains(&level)
                        }))
                    }
                };
                Reachability::Regions {
                    shape,
                    came_from: rect,
                    regions,
                }
            }
            (false, ComesFrom::Footprint { rect, layer, .. }) => {
                match relation(grid, layer, shape.layer, rect) {
                    relation @ (Relation::Same | Relation::Below) => Reachability::Heights {
                        shape,
                        came_from: rect,
                        levels: levels_beside(rect, relation),
                    },
                    Relation::Above => Reachability::Unchecked,
                }
            }
            (false, ComesFrom::Point(cell)) => {
                let point = CellRect::new(cell, CellSize::ONE);
                Reachability::Heights {
                    shape,
                    came_from: point,
                    levels: levels_beside(point, Relation::Same),
                }
            }
        }
    }

    /// Whether a body anchored at `cell` lands where it may.
    fn lands_on(&self, map: &Map, cell: CellPos) -> bool {
        match self {
            Reachability::Unchecked => true,
            Reachability::Heights {
                shape,
                came_from,
                levels,
            } => {
                let footprint = CellRect::new(cell, shape.size);
                // The ground it comes from is its own.
                footprint.intersects(*came_from)
                    || highest_level(map.nav_grid(), shape.layer, footprint)
                        .is_some_and(|level| levels.contains(&level))
            }
            Reachability::Regions {
                shape,
                came_from,
                regions,
            } => {
                // The ground it comes from is its own, whatever region its
                // cells were read in.
                CellRect::new(cell, shape.size).intersects(*came_from)
                    || map
                        .hierarchy()
                        .region_of(cell, *shape)
                        .is_some_and(|region| regions.contains(&region))
            }
        }
    }
}

/// Which levels a body of a shape on `layer` coming `from` there tries first:
/// out of a footprint on a layer above, the highest level of `layer` under it.
fn first_levels(grid: &NavGrid, layer: LayerId, from: ComesFrom) -> Levels {
    match from {
        ComesFrom::Footprint {
            rect,
            layer: source,
            ..
        } => match relation(grid, source, layer, rect) {
            Relation::Above => Levels::HighestFirst(
                highest_level(grid, layer, rect)
                    .expect("a footprint a body comes out of stands on the map"),
            ),
            Relation::Same | Relation::Below => Levels::Any,
        },
        ComesFrom::Point(_) => Levels::Any,
    }
}

/// Where a footprint on `source` stands against `layer` over `rect`: above
/// when the source's highest level there lies over the layer's.
///
/// Panics if `rect` runs off the map.
fn relation(grid: &NavGrid, source: LayerId, layer: LayerId, rect: CellRect) -> Relation {
    if source == layer {
        return Relation::Same;
    }
    let level = |layer| {
        highest_level(grid, layer, rect).expect("a footprint a body comes out of stands on the map")
    };
    if level(source) > level(layer) {
        Relation::Above
    } else {
        Relation::Below
    }
}

/// Every anchor of a `size` footprint within `reach` cells of `rect` by its
/// bounding square, the footprint on the map: none for one wider than it.
fn anchors_within(
    grid: &NavGrid,
    rect: CellRect,
    size: CellSize,
    reach: u32,
) -> impl Iterator<Item = CellPos> {
    let min_x = rect
        .origin
        .x
        .saturating_sub((size.width - 1).saturating_add(reach));
    let min_y = rect
        .origin
        .y
        .saturating_sub((size.height - 1).saturating_add(reach));
    // A footprint wider than the map has no anchor on it.
    let window = match (
        grid.width().checked_sub(size.width),
        grid.height().checked_sub(size.height),
    ) {
        (Some(last_x), Some(last_y)) => Some((
            min_y
                ..=(rect.origin.y + rect.size.height - 1)
                    .saturating_add(reach)
                    .min(last_y),
            min_x
                ..=(rect.origin.x + rect.size.width - 1)
                    .saturating_add(reach)
                    .min(last_x),
        )),
        (None, _) | (_, None) => None,
    };
    window.into_iter().flat_map(|(rows, columns)| {
        rows.flat_map(move |y| columns.clone().map(move |x| CellPos::new(x, y)))
    })
}

/// The highest level of `layer` under `rect`, or `None` off the map.
fn highest_level(grid: &NavGrid, layer: LayerId, rect: CellRect) -> Option<u32> {
    rect.cells().try_fold(None, |highest: Option<u32>, cell| {
        let level = grid.relief(layer, cell)?.level;
        Some(Some(highest.map_or(level, |highest| highest.max(level))))
    })?
}

/// The layer `entity`'s type stands on.
///
/// Panics if `entity` is not a simulation entity, or its type declares no
/// location.
fn layer_of(world: &World, entity: Entity) -> LayerId {
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only an entity that stands somewhere sets a body down");
    entity_def::mover_shape(world.resource::<ContentRegistry>(), &location_def).layer
}

/// `container` when it stands on the map, else the first that stands of what
/// it is inside, else itself.
fn standing_container(world: &World, container: Entity) -> Entity {
    if world.entity(container).contains::<LocationComponent>() {
        return container;
    }
    match entity_def::inside(world, container) {
        Some(outer) => standing_container(world, outer),
        None => container,
    }
}

/// How many rings out from `around` the farthest cell of `rect` lies.
fn rings_to(around: CellRect, rect: CellRect) -> u32 {
    // On one axis: how far `rect`'s span `from..=through` reaches past the
    // span of `around` that starts at `low`, `extent` long.
    let axis = |low: u32, extent: u32, from: u32, through: u32| {
        let high = low + extent - 1;
        low.saturating_sub(from).max(through.saturating_sub(high))
    };
    axis(
        around.origin.x,
        around.size.width,
        rect.origin.x,
        rect.origin.x + rect.size.width - 1,
    )
    .max(axis(
        around.origin.y,
        around.size.height,
        rect.origin.y,
        rect.origin.y + rect.size.height - 1,
    ))
}
