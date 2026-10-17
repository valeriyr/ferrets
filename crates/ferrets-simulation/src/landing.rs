//! Where a body set down away from where it stood may land: ground that takes
//! its footprint, admitted by its type's placement rules, and seen as the
//! landing word asks by the team that sets it down.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{entity_type_def::EntityTypeDef, landing::Landing};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{
    components::location::LocationComponent,
    fields,
    ground::{self, UnderfootBody},
    map::{GridMark, Map, OccupancyClass},
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
    /// Bodies set down for `player` as `landing` says, the ones being moved
    /// marking `lifted` and being `ignored`; nothing given out yet.
    pub(crate) fn new(
        player: PlayerId,
        landing: Landing,
        lifted: &'a [GridMark],
        ignored: &'a [Entity],
    ) -> Self {
        Self {
            player,
            landing,
            lifted,
            ignored,
            underfoot: None,
            taken: Vec::new(),
        }
    }
}

/// Whether `player` sees every cell of `footprint`, all on the map, as
/// `landing` asks, by the vision its seat declares: an omniscient seat sees
/// every cell, a filtered one what its team's fog grid shows, a free seat
/// none.
pub(crate) fn sees(world: &World, player: PlayerId, landing: Landing, footprint: CellRect) -> bool {
    let session = world.resource::<GameSession>();
    let vision = session
        .slot(player)
        .and_then(PlayerSlot::senses)
        .map(|(vision, _)| vision);
    let seen = |enough: fn(CellVisibility) -> bool| match vision {
        Some(AiVision::Omniscient) => true,
        Some(AiVision::Filtered) => {
            let grid = world.resource::<VisibilityGrid>();
            footprint
                .cells()
                .all(|cell| enough(grid.visibility_to(session, player, cell.x, cell.y)))
        }
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
/// its type's placement rules, and seen as `landings` asks. Its footprint is
/// then given out. `None` when nothing within the placement search radius will
/// do, and nothing is given out.
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
    let size = location_def.size();
    let underfoot: &[UnderfootBody] = match class {
        OccupancyClass::Static => landings
            .underfoot
            .get_or_insert_with(|| ground::standing_underfoot(world, landings.ignored)),
        OccupancyClass::Claim => &[],
    };
    let cell = world.resource::<Map>().find_placement_near_toward_where(
        around.origin,
        around.size,
        size,
        toward,
        |cell| {
            let footprint = CellRect::new(cell, size);
            let placed = LocationComponent::new(FixedUVec2::from(cell), spawn::DEFAULT_FACING);
            // The ground before sight: it refuses a footprint running off the
            // map, every cell of which `sees` reads.
            !landings
                .taken
                .iter()
                .any(|taken| taken.intersects(footprint))
                && ground::can_place_clear_of(
                    world,
                    &placed,
                    location_def,
                    class,
                    landings.lifted,
                    underfoot,
                )
                && sees(world, landings.player, landings.landing, footprint)
                && fields::allows_placement(world, owner, def, cell)
        },
    )?;
    landings.taken.push(CellRect::new(cell, size));
    Some(cell)
}
