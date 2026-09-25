//! What the placement ghost judges, and what an overbuilt source must be for it to snap to it.

mod utils;

use bevy::prelude::*;
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::{input, render::Sighted};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    components::{entity_info::EntityInfoComponent, location::LocationComponent},
    fields::FieldGrid,
    map::Map,
    movement_model::MovementModel,
    session::GameSession,
    visibility::Sighting,
};

//
// ─── Placement ghost ──────────────────────────────────────────────────────────
//

#[test]
fn ghost_fits_over_burrowed_enemy_and_refuses_claimed_cell() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    // A rival's burrowed swarmling at (22, 22) claims no cell, and the ghost
    // never asks what stands underfoot: a barracks anchored at (20, 20), whose
    // footprint reaches (22, 22), shows as fitting, and tells nothing of it.
    // The raise refuses it when it starts. A rival's barracks at (25, 25)
    // claims its cells, and the ghost anchored there shows as refused.
    utils::create_entity(
        app.world_mut(),
        "swarmling_burrowed",
        utils::at_cell(22, 22),
        Some(1),
    )
    .expect("the demo content defines a burrowed swarmling");
    utils::create_entity(app.world_mut(), "barracks", utils::at_cell(25, 25), Some(1))
        .expect("the demo content defines a barracks");

    assert!(
        fits(&app, "barracks", CellPos::new(20, 20)),
        "the ghost reveals no burrowed enemy"
    );
    assert!(
        !fits(&app, "barracks", CellPos::new(25, 25)),
        "the ghost refuses what claims its cells"
    );
}

//
// ─── Overbuild snap ───────────────────────────────────────────────────────────
//

#[test]
fn ghost_snaps_to_seen_source_under_cursor_and_to_no_other() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    // The mine spans (10, 10) to (11, 11): a cursor on its far cell snaps to
    // its origin, one beside it to nothing.
    let (mine, _) =
        utils::create_entity(app.world_mut(), "gold_mine", utils::at_cell(10, 10), None)
            .expect("the demo content defines a gold mine");

    let seen = Sighted(Sighting::Seen);
    assert_eq!(
        snap(&app, mine, "gold_mine", CellPos::new(11, 11), &seen),
        Some(CellPos::new(10, 10))
    );
    assert_eq!(
        snap(&app, mine, "gold_mine", CellPos::new(12, 11), &seen),
        None
    );
    assert_eq!(
        snap(&app, mine, "barracks", CellPos::new(11, 11), &seen),
        None,
        "only a source of the overbuilt type snaps"
    );
    assert_eq!(
        snap(
            &app,
            mine,
            "gold_mine",
            CellPos::new(11, 11),
            &Sighted(Sighting::Glimpsed)
        ),
        None,
        "a source the perspective does not see is not snapped to"
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Whether the ghost of `type_name` anchored at the cell shows as fitting for
/// player 0.
fn fits(app: &App, type_name: &str, anchor: CellPos) -> bool {
    let world = app.world();
    let def = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("the demo content defines the type");
    input::ghost_fits(
        world.resource::<Map>(),
        world.resource::<FieldGrid>(),
        world.resource::<GameSession>(),
        0,
        def,
        anchor,
    )
}

/// What the ghost snaps to for `source` under `cursor`, the source stamped as
/// `sighted` and drawn.
fn snap(
    app: &App,
    source: Entity,
    over: &str,
    cursor: CellPos,
    sighted: &Sighted,
) -> Option<CellPos> {
    let world = app.world();
    let info = world
        .get::<EntityInfoComponent>(source)
        .expect("a placed source");
    let location = world
        .get::<LocationComponent>(source)
        .expect("a placed source");
    let standing = world.resource::<ContentRegistry>().def(info.type_id());
    input::snaps_to(
        over,
        cursor,
        info,
        location,
        standing,
        &Visibility::Visible,
        Some(sighted),
    )
}
