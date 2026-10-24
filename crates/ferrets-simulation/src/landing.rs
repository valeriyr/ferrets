//! Where a body set down away from where it stood may land: ground that takes
//! its footprint — over a source of its type, for a type raised over one —
//! admitted by its type's placement rules and the field rules it is held to,
//! and seen as the landing word asks by the team that sets it down.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_type_def::EntityTypeDef, field::FieldPlacement, landing::Landing,
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{
    components::location::LocationComponent,
    fields,
    ground::{self, UnderfootBody},
    map::Map,
    occupancy::{GridMark, OccupancyClass},
    overbuild::{self, Cover},
    session::{GameSession, ai_vision::AiVision, player_id::PlayerId, player_slot::PlayerSlot},
    spawn,
    visibility::{CellVisibility, VisibilityGrid},
};

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
    /// them stands underfoot of another.
    ignored: &'a [Entity],
    /// What stands underfoot, none of `ignored` counted: `None` until a body
    /// on the static plane first lands, then kept for the rest.
    underfoot: Option<Vec<UnderfootBody>>,
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
            underfoot: None,
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

/// The anchor nearest `around` that a body of `def`, owned by `owner`, lands
/// on among `landings`, ring by ring outward and within each ring the cells
/// nearest `toward` first: ground that takes its footprint over what the
/// bodies moved together hold, clear of every footprint given out, admitted by
/// its type's placement rules and the field rules of `landings`, and seen as
/// `landings` asks. Its footprint is then given out. `None` when nothing within
/// the placement search radius will do, and nothing is given out.
///
/// Panics for a type that stands nowhere.
pub(crate) fn land(
    world: &World,
    landings: &mut Landings,
    def: &EntityTypeDef,
    owner: Option<PlayerId>,
    around: CellRect,
    toward: CellRect,
) -> Option<CellPos> {
    let location_def = def
        .location
        .expect("only a type that stands somewhere lands");
    let class = OccupancyClass::of(def);
    // What stands underfoot is read once for every body set down together,
    // and lent back to `landings` after this one.
    let underfoot: Vec<UnderfootBody> = match class {
        OccupancyClass::Static => landings
            .underfoot
            .take()
            .unwrap_or_else(|| ground::standing_underfoot(world, landings.ignored)),
        OccupancyClass::Claim => Vec::new(),
    };
    let lifted = landings.lifted;
    let cell = land_on(world, landings, def, owner, around, toward, |cell, _| {
        let placed = LocationComponent::new(FixedUVec2::from(cell), spawn::DEFAULT_FACING);
        ground::can_place_clear_of(world, &placed, location_def, class, lifted, &underfoot)
    });
    match class {
        OccupancyClass::Static => landings.underfoot = Some(underfoot),
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
    def: &EntityTypeDef,
    over: &str,
    owner: Option<PlayerId>,
    around: CellRect,
) -> Option<(CellPos, Entity)> {
    let location_def = def
        .location
        .expect("only a type that stands somewhere lands");
    let class = OccupancyClass::of(def);
    let (landings_lifted, landings_ignored) = (landings.lifted, landings.ignored);
    let cell = land_on(
        world,
        landings,
        def,
        owner,
        around,
        around,
        |cell, footprint| match overbuild::source_under(world, over, footprint) {
            Some(source) => {
                let (mut lifted, mut ignored) = overbuild::set_aside(world, Cover::Source(source));
                lifted.extend_from_slice(landings_lifted);
                ignored.extend_from_slice(landings_ignored);
                let placed = LocationComponent::new(FixedUVec2::from(cell), spawn::DEFAULT_FACING);
                ground::can_place(world, &placed, location_def, class, &lifted, &ignored)
            }
            None => false,
        },
    )?;
    let footprint = CellRect::new(cell, location_def.size());
    let source = overbuild::source_under(world, over, footprint)
        .expect("a cell is landed over only where a source stands");
    Some((cell, source))
}

/// The anchor nearest `around` for a body of `def` among `landings`, in
/// [`land`]'s order, whose footprint `takes` admits as ground, clear of every
/// footprint given out, admitted by its type's placement rules and the field
/// rules of `landings`, and seen as `landings` asks; its footprint is then
/// given out.
fn land_on(
    world: &World,
    landings: &mut Landings,
    def: &EntityTypeDef,
    owner: Option<PlayerId>,
    around: CellRect,
    toward: CellRect,
    takes: impl Fn(CellPos, CellRect) -> bool,
) -> Option<CellPos> {
    let size = def
        .location
        .expect("only a type that stands somewhere lands")
        .size();
    fields::debug_assert_spreads_on(
        world.resource::<ContentRegistry>(),
        def,
        landings.within.iter().map(FieldPlacement::field),
    );
    let cell = world.resource::<Map>().find_placement_near_toward_where(
        around.origin,
        around.size,
        size,
        toward,
        |cell| {
            let footprint = CellRect::new(cell, size);
            // The ground before sight: it refuses a footprint running off the
            // map, every cell of which `sees` reads.
            !landings
                .taken
                .iter()
                .any(|taken| taken.intersects(footprint))
                && takes(cell, footprint)
                && sees(world, landings.player, landings.landing, footprint)
                && fields::rules_allow(world, Some(landings.player), landings.within, footprint)
                && fields::allows_placement(world, owner, def, cell)
        },
    )?;
    landings.taken.push(CellRect::new(cell, size));
    Some(cell)
}
