//! What the placement ghost judges and snaps to, and what a click asks of a selection.

mod utils;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::PendingInput;
use ferrets_content::{
    affiliation::Affiliation,
    field::{FieldCoverage, FieldPlacement},
    registry::ContentRegistry,
};
use ferrets_demo::{input, render::Sighted};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        entity_info::EntityInfoComponent,
        location::LocationComponent,
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        rise::RisingComponent,
        warp::WarpComponent,
    },
    fields::{self, FieldGrid},
    map::Map,
    movement_model::MovementModel,
    resources::PlayerResources,
    selection::Selection,
    session::GameSession,
    simulation_id::SimulationId,
    visibility::{Sighting, VisibilityGrid},
};

//
// ─── Placement ghost ──────────────────────────────────────────────────────────
//

#[test]
fn ghost_fits_over_burrowed_enemy_and_refuses_claimed_cell_made_out() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    // A rival's burrowed swarmling at (22, 22) claims no cell, and the ghost
    // never asks what stands underfoot: a barracks anchored at (20, 20), whose
    // footprint reaches (22, 22), shows as fitting, and tells nothing of it.
    // The raise refuses it when it starts. A rival's barracks at (25, 25),
    // made out, claims its cells, and the ghost anchored there shows as
    // refused.
    let (swarmling, _) = utils::create_entity(
        app.world_mut(),
        "swarmling_burrowed",
        utils::at_cell(22, 22),
        Some(1),
    )
    .expect("the demo content defines a burrowed swarmling");
    let (barracks, _) =
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(25, 25), Some(1))
            .expect("the demo content defines a barracks");

    assert!(
        fits(
            &app,
            "barracks",
            CellPos::new(20, 20),
            &[swarmling, barracks]
        ),
        "the ghost reveals no burrowed enemy"
    );
    assert!(
        !fits(
            &app,
            "barracks",
            CellPos::new(25, 25),
            &[swarmling, barracks]
        ),
        "the ghost refuses what claims its cells"
    );
}

#[test]
fn ghost_fits_over_what_side_does_not_make_out() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    utils::create_entity(app.world_mut(), "barracks", utils::at_cell(25, 25), Some(1))
        .expect("the demo content defines a barracks");

    // The rival's barracks claims the cells, but nothing of it is made out:
    // the ghost shows the ground as the side knows it, and the click's
    // refusal tells the rest.
    assert!(fits(&app, "barracks", CellPos::new(25, 25), &[]));
}

#[test]
fn ghost_judges_field_rules_on_coverage_overlay_shows() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    // A rival's pylon powers its cells; the overlay shows only the player's
    // own power.
    utils::create_entity(app.world_mut(), "pylon", utils::at_cell(20, 20), Some(1))
        .expect("the demo content defines a pylon");
    utils::run_ticks(&mut app, 1);
    let power = app
        .world()
        .resource::<ContentRegistry>()
        .field("power")
        .expect("the demo content defines power");
    let on_enemy_power = [FieldPlacement::Requires {
        field: power,
        of: Affiliation::Enemy,
        coverage: FieldCoverage::Every,
    }];
    let rect = CellRect::new(CellPos::new(21, 21), CellSize::ONE);

    // The simulation sees the rival's power there; the ghost, shown none of
    // it, judges the rule unmet.
    assert!(fields::rules_allow_in(
        app.world().resource::<FieldGrid>(),
        app.world().resource::<GameSession>(),
        Some(0),
        &on_enemy_power,
        rect,
    ));
    assert!(!with_view(&app, &[], |view| view.allows(&on_enemy_power, rect)));
}

#[test]
fn ghost_reads_no_rival_field_in_cells_side_does_not_see() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    // A rival's hatchery spreads creep about itself; nothing of player 0's
    // sees there yet.
    utils::create_entity(app.world_mut(), "hatchery", utils::at_cell(30, 30), Some(1))
        .expect("the demo content defines a hatchery");
    utils::run_ticks(&mut app, 2);
    let creep = app
        .world()
        .resource::<ContentRegistry>()
        .field("creep")
        .expect("the demo content defines creep");
    let off_creep = [FieldPlacement::Forbids { field: creep }];
    let rect = CellRect::new(CellPos::new(31, 34), CellSize::ONE);
    assert!(!fields::rules_allow_in(
        app.world().resource::<FieldGrid>(),
        app.world().resource::<GameSession>(),
        Some(0),
        &off_creep,
        rect,
    ));
    // Unseen, the rival's creep is not the ghost's to read.
    assert!(with_view(&app, &[], |view| view.allows(&off_creep, rect)));

    // Seen, it is.
    utils::create_entity(app.world_mut(), "zealot", utils::at_cell(31, 36), Some(0))
        .expect("the demo content defines a zealot");
    utils::run_ticks(&mut app, 1);
    assert!(!with_view(&app, &[], |view| view.allows(&off_creep, rect)));
}

#[test]
fn ghost_is_blocked_only_by_what_side_makes_out() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    let (barracks, _) =
        utils::create_entity(app.world_mut(), "barracks", utils::at_cell(25, 25), Some(1))
            .expect("the demo content defines a barracks");
    let world = app.world();
    let info = world.get::<EntityInfoComponent>(barracks).expect("placed");
    let location = world.get::<LocationComponent>(barracks).expect("placed");
    let registry = world.resource::<ContentRegistry>();
    let def = registry
        .entity("barracks")
        .expect("the demo content defines a barracks");
    let glimpsed = Sighted(Sighting::Glimpsed);
    let seen = Sighted(Sighting::Seen);

    // Glimpsed, or not drawn, it is not made out and a ghost on its cells
    // fits; seen and drawn, it is, and the ghost does not.
    for (visibility, sighted, fits) in [
        (Visibility::Visible, &glimpsed, true),
        (Visibility::Hidden, &seen, true),
        (Visibility::Visible, &seen, false),
    ] {
        let made_out = input::made_out_footprints(
            registry,
            [(info, location, &visibility, Some(sighted))].into_iter(),
        );
        assert_eq!(
            view_over(&app, &made_out, |view| view.fits(def, CellPos::new(25, 25))),
            fits,
            "{visibility:?}"
        );
    }
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

#[test]
fn ghost_snapped_to_source_fits_only_as_its_field_rules_say() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "gold_mine", utils::at_cell(10, 10), None)
            .expect("the demo content defines a gold mine");
    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let creep = registry
        .field("creep")
        .expect("the demo content defines creep");
    // A refinery that must stand on creep, where no creep lies.
    let mut def = registry
        .entity("refinery")
        .expect("the demo content defines a refinery")
        .clone();
    def.field_placement = vec![FieldPlacement::Requires {
        field: creep,
        of: Affiliation::Anyone,
        coverage: FieldCoverage::Every,
    }];
    let info = world.get::<EntityInfoComponent>(mine).expect("placed");
    let location = world.get::<LocationComponent>(mine).expect("placed");
    let seen = Sighted(Sighting::Seen);

    assert_eq!(
        with_view(&app, &[], |view| view.placed(
            &def,
            CellPos::new(11, 11),
            [(info, location, &Visibility::Visible, Some(&seen))]
        )),
        (CellPos::new(10, 10), false)
    );
}

#[test]
fn ghost_of_type_raised_over_source_stands_on_seen_source_under_cursor() {
    let mut app = utils::demo_map_app(MovementModel::Continuous);
    let (mine, _) =
        utils::create_entity(app.world_mut(), "gold_mine", utils::at_cell(10, 10), None)
            .expect("the demo content defines a gold mine");
    let world = app.world();
    let def = world
        .resource::<ContentRegistry>()
        .entity("refinery")
        .expect("the demo content defines a refinery");
    let info = world.get::<EntityInfoComponent>(mine).expect("placed");
    let location = world.get::<LocationComponent>(mine).expect("placed");
    let seen = Sighted(Sighting::Seen);
    let sources = || [(info, location, &Visibility::Visible, Some(&seen))];

    // On the mine's far cell it stands on the mine and fits; beside the mine,
    // on open ground, it stays under the cursor and does not.
    assert_eq!(
        with_view(&app, &[], |view| view.placed(
            def,
            CellPos::new(11, 11),
            sources()
        )),
        (CellPos::new(10, 10), true)
    );
    assert_eq!(
        with_view(&app, &[], |view| view.placed(
            def,
            CellPos::new(20, 20),
            sources()
        )),
        (CellPos::new(20, 20), false)
    );
}

//
// ─── Right-click ──────────────────────────────────────────────────────────────
//

#[test]
fn right_click_sets_rally_of_rising_mover_and_moves_standing_one() {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    let (_, rising_id) =
        utils::create_rising(app.world_mut(), "zealot", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a zealot");
    let (_, standing) =
        utils::create_entity(app.world_mut(), "zealot", utils::at_cell(24, 20), Some(0))
            .expect("the demo content defines a zealot");

    // A mover still rising moves only once it has risen: the click names where
    // it goes then.
    utils::select(&mut app, rising_id, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY);
    let commands = right_click(&mut app);
    assert!(matches!(
        commands.as_slice(),
        [PlayerCommand::SetRallyPoint { entity, target: Some(RallyTarget::Position(_)) }]
            if *entity == rising_id
    ));

    utils::select(&mut app, standing, SelectMode::Replace);
    utils::run_ticks(&mut app, utils::APPLY);
    let commands = right_click(&mut app);
    assert!(matches!(commands.as_slice(), [PlayerCommand::Move { .. }]));
}

//
// ─── Warp click ───────────────────────────────────────────────────────────────
//

#[test]
fn warp_click_goes_to_first_selected_gate_that_may_warp() {
    let mut app = warp_app();
    let (cooling, cooling_id) = warp_gate(&mut app, 20, Some(0));
    let (_, ready_id) = warp_gate(&mut app, 26, Some(0));
    utils::run_ticks(&mut app, 1);
    // The first gate selected still cools down from a warp of its own.
    app.world_mut()
        .get_mut::<WarpComponent>(cooling)
        .expect("a warp gate keeps its cooldown")
        .ready_at = u32::MAX;
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![cooling_id, ready_id]);

    let commands = click_warp(&mut app, cooling_id);
    assert!(matches!(
        commands.as_slice(),
        [PlayerCommand::WarpIn { warper, .. }] if *warper == ready_id
    ));
}

#[test]
fn second_warp_click_before_landing_goes_to_next_ready_gate() {
    let mut app = warp_app();
    let (_, first) = warp_gate(&mut app, 20, Some(0));
    let (_, second) = warp_gate(&mut app, 26, Some(0));
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![first, second]);

    // Each click within the same tick goes to a gate not yet given one; with
    // both given one, the third goes to the leader, whose refusal the
    // simulation answers.
    let warpers: Vec<SimulationId> = (0..3)
        .flat_map(|_| click_warp(&mut app, first))
        .map(|command| match command {
            PlayerCommand::WarpIn { warper, .. } => warper,
            other => panic!("a warp click gives only a warp-in, not {other:?}"),
        })
        .collect();
    assert_eq!(warpers, vec![first, second, first]);
}

#[test]
fn warp_click_offers_leader_first() {
    let mut app = warp_app();
    let (_, leader) = warp_gate(&mut app, 20, Some(0));
    let (_, other) = warp_gate(&mut app, 26, Some(0));
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![other, leader]);

    let commands = click_warp(&mut app, leader);
    assert!(matches!(
        commands.as_slice(),
        [PlayerCommand::WarpIn { warper, .. }] if *warper == leader
    ));
}

#[test]
fn sent_warp_is_in_flight_until_its_tick_has_surely_run() {
    let sent = input::SentWarp {
        warper: SimulationId(1),
        tick: 10,
        cooldown_end: 0,
    };
    // Sent at tick 10, its command lands by tick 10 + SYNC_LATENCY (2) = 12
    // and has surely run by the tick after it, 13.
    assert_eq!(
        (sent.in_flight(10), sent.in_flight(13), sent.in_flight(14)),
        (true, true, false)
    );
}

#[test]
fn warp_candidates_return_to_gate_whose_cooldown_moved() {
    let mut app = warp_app();
    let (first, first_id) = warp_gate(&mut app, 20, Some(0));
    let (second, second_id) = warp_gate(&mut app, 26, Some(0));
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![first_id, second_id]);
    let tick = app.world().resource::<GameSession>().tick();
    let ready_at = app
        .world()
        .get::<WarpComponent>(first)
        .expect("a warp gate keeps its cooldown")
        .ready_at;
    let mut sent = |cooldown_end| {
        app.world_mut().resource_mut::<input::WarpsSent>().0 = vec![input::SentWarp {
            warper: first_id,
            tick,
            cooldown_end,
        }];
        input::warp_candidates(app.world(), 0, first_id)
    };

    // Given a warp-in this tick, the first gate is passed over while its
    // cooldown holds as it was; once the cooldown has moved — its command
    // landed — it is offered again.
    assert_eq!(sent(ready_at), vec![second]);
    assert_eq!(sent(ready_at + 1), vec![first, second]);
}

#[test]
fn warp_click_skips_rival_gate_in_selection() {
    let mut app = warp_app();
    let (cooling, leader) = warp_gate(&mut app, 20, Some(0));
    let (_, rival) = warp_gate(&mut app, 30, Some(1));
    // The rival's own pylon powers its gate, which stands ready.
    utils::create_entity(app.world_mut(), "pylon", utils::at_cell(32, 23), Some(1))
        .expect("the demo content defines a pylon");
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .get_mut::<WarpComponent>(cooling)
        .expect("a warp gate keeps its cooldown")
        .ready_at = u32::MAX;
    app.world_mut()
        .resource_mut::<Selection>()
        .set(0, vec![leader, rival]);

    // Only the player's own gates are offered the warp-in: the cooling leader
    // is given it.
    let commands = click_warp(&mut app, leader);
    assert!(matches!(
        commands.as_slice(),
        [PlayerCommand::WarpIn { warper, .. }] if *warper == leader
    ));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Whether the ghost of `type_name` anchored at the cell shows as fitting for
/// player 0, who makes out `made_out` alone.
fn fits(app: &App, type_name: &str, anchor: CellPos, made_out: &[Entity]) -> bool {
    let def = app
        .world()
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("the demo content defines the type");
    with_view(app, made_out, |view| view.fits(def, anchor))
}

/// What `judge` answers of player 0's placement ghost view, `made_out` the
/// only entities it makes out.
fn with_view<R>(app: &App, made_out: &[Entity], judge: impl FnOnce(&input::GhostView) -> R) -> R {
    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let seen = Sighted(Sighting::Seen);
    let entities = made_out.iter().map(|&entity| {
        let info = world
            .get::<EntityInfoComponent>(entity)
            .expect("a placed entity");
        let location = world
            .get::<LocationComponent>(entity)
            .expect("a placed entity");
        (info, location, &Visibility::Visible, Some(&seen))
    });
    let footprints = input::made_out_footprints(registry, entities);
    view_over(app, &footprints, judge)
}

/// What `judge` answers of player 0's placement ghost view, `made_out` the
/// only footprints it makes out.
fn view_over<R>(
    app: &App,
    made_out: &[(LayerMask, CellRect)],
    judge: impl FnOnce(&input::GhostView) -> R,
) -> R {
    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    judge(&input::GhostView {
        map: world.resource::<Map>(),
        registry,
        fields: world.resource::<FieldGrid>(),
        fog: world.resource::<VisibilityGrid>(),
        session: world.resource::<GameSession>(),
        player: 0,
        made_out,
    })
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

/// The commands a right-click on open ground asks of the local selection.
fn right_click(app: &mut App) -> Vec<PlayerCommand> {
    app.world_mut().insert_resource(PendingInput::default());
    app.world_mut()
        .run_system_once(
            |session: Res<GameSession>,
             selection: Res<Selection>,
             registry: Res<ContentRegistry>,
             holders: Query<(
                &EntityInfoComponent,
                &OwnerComponent,
                Option<&RallyPointComponent>,
                Has<RisingComponent>,
            )>,
             mut pending: ResMut<PendingInput>| {
                input::issue_orders_at(
                    Vec2::new(28.0 * 32.0, -20.0 * 32.0),
                    None,
                    true,
                    &session,
                    &selection,
                    &registry,
                    &holders,
                    &mut pending,
                );
            },
        )
        .expect("the right-click runs");
    app.world().resource::<PendingInput>().queued().to_vec()
}

/// The demo map with 500 gold for player 0, a pylon powering warp gates
/// placed along row 20, and no warp-in clicked or given yet.
fn warp_app() -> App {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    // Warp gates stand idle unpowered.
    utils::create_entity(app.world_mut(), "pylon", utils::at_cell(24, 23), Some(0))
        .expect("the demo content defines a pylon");
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 500);
    app.world_mut().insert_resource(PendingInput::default());
    app.world_mut().init_resource::<input::WarpsSent>();
    app
}

/// A warp gate of `owner` at `(x, 20)`.
fn warp_gate(app: &mut App, x: u32, owner: Option<u8>) -> (Entity, SimulationId) {
    utils::create_entity(app.world_mut(), "warp_gate", utils::at_cell(x, 20), owner)
        .expect("the demo content defines a warp gate")
}

/// Clicks a zealot's warp-in led by `leading`, and returns the commands the
/// click gave.
fn click_warp(app: &mut App, leading: SimulationId) -> Vec<PlayerCommand> {
    let before = app.world().resource::<PendingInput>().queued().len();
    app.world_mut()
        .insert_resource(input::WarpClick(Some(input::Warp {
            leading,
            type_name: "zealot".into(),
            at: utils::at_cell(22, 24),
        })));
    app.world_mut()
        .run_system_once(input::issue_warp)
        .expect("the warp click runs");
    app.world().resource::<PendingInput>().queued()[before..].to_vec()
}
