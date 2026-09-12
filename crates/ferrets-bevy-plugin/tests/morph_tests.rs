//! Form changes on the engine's own content: the grid swaps, the pools, and
//! the payment, staged on synthetic types so the mechanics are pinned apart
//! from any game's balance.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        attached::AttachedComponent,
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        rally::RallyTarget,
        resource::ResourceSourceComponent,
        train::TrainQueueComponent,
    },
    entity_def,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── The grid swap ─────────────────────────────────────────────────────────────
//

#[test]
fn same_layer_growth_lands_under_continuous_model() {
    // Growing 1x1 -> 3x3 on the same layer recentres the anchor onto ground
    // the whelp's own claim covers: the change must lift that claim out of
    // the destination's way — under the continuous model displacing it is a
    // rebuild-owned no-op, so the lift has to take the claim where the
    // rebuilt plane holds it.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    // One tick so the rebuilt claim plane holds the whelp's footprint.
    utils::run_ticks(&mut app, 1);

    order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn same_layer_growth_lands_under_cell_model() {
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn odd_growth_settles_on_lattice_under_cell_model() {
    // 1x1 -> 2x2 recentres the anchor by half a cell, to 9.5; the cell model
    // keeps every body on the lattice, so the ogre settles on the nearest
    // point, 10, and holds cells 10..=11 — at rest, free to move on.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    order_morph(&mut app, whelp, "ogre");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "ogre");
    assert_eq!(utils::position_of(app.world(), whelp), utils::pos(10, 10));
    utils::select(&mut app, whelp_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(15, 15),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_ne!(utils::position_of(app.world(), whelp), utils::pos(10, 10));
}

#[test]
fn odd_growth_keeps_middle_under_continuous_model() {
    // Continuous bodies rest anywhere: the ogre settles at the recentred
    // 9.5, its middle exactly where the whelp's was.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::run_ticks(&mut app, 1);

    order_morph(&mut app, whelp, "ogre");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "ogre");
    assert_eq!(
        utils::position_of(app.world(), whelp),
        FixedUVec2::new(FixedU64::from_num(9.5), FixedU64::from_num(9.5))
    );
}

#[test]
fn unrooting_swaps_static_footprint_for_claim() {
    // Same occupation, same size — only the plane changes: the shrine's
    // static footprint must come off the grid and the golem's claim go on,
    // or the ghost of the building walls its own unit in forever.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, _) = utils::create_owned(&mut app, "shrine", 10, 10, 0);

    order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, shrine), "golem");
    let world = app.world();
    let grid = world.resource::<Map>().nav_grid();
    for cell in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        let cell = CellPos::new(cell.0, cell.1);
        assert!(
            grid.is_statically_passable_by(utils::GROUND, cell),
            "the shrine's static footprint survived its unrooting at {cell:?}"
        );
        assert!(
            grid.is_claimed_by(utils::GROUND, cell),
            "the golem claims the ground it stands on at {cell:?}"
        );
    }
}

//
// ─── The pools and the payment ─────────────────────────────────────────────────
//

#[test]
fn instant_change_pays_from_old_pools() {
    // A blood price on an instant change: the cost leaves the OLD form's
    // pool before the landing rescales it. Paying after would draw the full
    // price from the husk's small pool and kill what the affordability
    // check promised would survive.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    order_morph(&mut app, whelp, "husk");
    utils::run_ticks(&mut app, 3);

    assert_eq!(type_name_of(&app, whelp), "husk");
    // (30 - 10) / 30 of the husk's 10 maximum, in binary fixed-point.
    assert_eq!(
        app.world()
            .entity(whelp)
            .get::<HealthComponent>()
            .expect("the husk keeps a health pool")
            .current(),
        FixedU64::from_num(10) * (FixedU64::from_num(20) / FixedU64::from_num(30)),
        "the blood price was drawn from the wrong form's pool"
    );
}

#[test]
fn zero_time_change_dying_on_refused_landing_dies_for_good() {
    // The boulder needs 3x3 of ground and every cell but the whelp's own is
    // blocked, so the landing is refused the tick the change begins; the
    // transition dies on interruption, and the whelp is gone two ticks of
    // dying later — with its Die order given, not left dying forever.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);

    order_morph(&mut app, whelp, "boulder");
    utils::run_ticks(&mut app, 1 + 2 + 1);

    utils::assert_despawned(app.world_mut(), whelp);
}

#[test]
fn changed_trainer_is_not_sent_to_its_own_rally_point() {
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, shrine_id) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: shrine_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The golem can walk, so a rally wrongly owed would carry it off; a
    // change of form sends nobody, and it stands where it unrooted.
    order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 11 + 5);

    assert_eq!(type_name_of(&app, shrine), "golem");
    assert!(entity_def::orders(app.world(), shrine).is_empty());
    assert_eq!(utils::position_of(app.world(), shrine), utils::pos(10, 10));
}

#[test]
fn form_without_pool_sheds_pool_component() {
    // The wisp declares no health: the pool component goes with the stat,
    // because a zero-maximum pool would read as dead rather than poolless.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "wisp");
    assert!(
        app.world().entity(whelp).get::<HealthComponent>().is_none(),
        "a poolless form kept a health pool"
    );
    assert!(
        app.world()
            .resource::<ferrets_simulation::entity_index::EntityIndex>()
            .alive(whelp_id)
            .is_some(),
        "shedding the pool must not read as dying"
    );
    assert!(
        app.world().entity(whelp).get::<EnergyComponent>().is_none(),
        "no form here carries energy"
    );
}

#[test]
fn form_gaining_pool_starts_it_full() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 15);
    assert_eq!(type_name_of(&app, whelp), "wisp");

    // Back into a form with health: the pool starts full — there is no old
    // proportion to carry when the old form had no pool at all.
    order_morph(&mut app, whelp, "whelp");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(
        app.world()
            .entity(whelp)
            .get::<HealthComponent>()
            .expect("the regained form has its pool back")
            .current(),
        FixedU64::from_num(30)
    );
}

//
// ─── Cancelling ────────────────────────────────────────────────────────────────
//

#[test]
fn queued_committed_change_drops_before_it_starts() {
    // A committed window refuses cancel only once it is open. Queued behind
    // a walk, the change has taken nothing and promised nothing — a soft
    // cancel drops it like any other waiting entry.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    app.world_mut()
        .entity_mut(whelp)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(20), FixedU64::from_num(10)),
                size: ferrets_geometry::cell_size::CellSize::ONE,
                range: 0,
            },
            None,
        );
    order_morph(&mut app, whelp, "husk");
    utils::run_ticks(&mut app, 2);

    app.world_mut()
        .entity_mut(whelp)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .cancel_all(CancelPolicy::Soft);
    utils::run_ticks(&mut app, 30);

    assert_eq!(
        type_name_of(&app, whelp),
        "whelp",
        "a queued committed change survived the cancel and landed"
    );
}

//
// ─── An interim form ──────────────────────────────────────────────────────────
//

#[test]
fn interim_form_is_worn_until_change_lands() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    order_morph(&mut app, whelp, "wyrm");

    // Paid and in its chrysalis the tick the change starts; a full whelp is
    // a full chrysalis.
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    assert_eq!(utils::gold(app.world()), 0);
    assert_eq!(utils::health(&app, whelp), 60);

    utils::run_ticks(&mut app, 12);
    assert_eq!(type_name_of(&app, whelp), "wyrm");
}

#[test]
fn interim_form_settles_onto_cell_mover_stood_on() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    // Part way across a cell boundary: the body rounds onto (11, 10) while the
    // position floors into (10, 10). A tick lets the claim plane follow.
    app.world_mut()
        .get_mut::<LocationComponent>(whelp)
        .unwrap()
        .position = utils::part_way("10.6", "10.3");
    utils::run_ticks(&mut app, 1);

    order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    assert_eq!(
        utils::position_of(app.world(), whelp),
        FixedUVec2::from(CellPos::new(11, 10)),
        "the chrysalis settles onto the cell it holds, not the fraction the whelp walked on"
    );
    let map = app.world().resource::<Map>();
    assert!(
        map.nav_grid()
            .is_statically_occupied_by(utils::GROUND, CellPos::new(11, 10)),
        "the chrysalis holds the cell the whelp stood on"
    );
    assert!(
        !map.nav_grid()
            .is_statically_occupied_by(utils::GROUND, CellPos::new(10, 10)),
        "and not the cell its position floored into"
    );
}

#[test]
fn cancelled_change_takes_interim_form_off() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    utils::stop_orders(app.world_mut(), whelp);
    utils::run_ticks(&mut app, 1);

    // Back to a whelp with the price returned.
    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn dying_entity_keeps_interim_form() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    spawn::destroy_entity(app.world_mut(), whelp);
    utils::run_ticks(&mut app, 1);

    // The cancelled change does not dress the corpse as a whelp again.
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
}

#[test]
fn fizzled_change_takes_interim_form_off() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    // Standing where the wyrm's footprint would spread, so the revalidating
    // landing is refused.
    utils::create_owned(&mut app, "whelp", 11, 10, 0);
    utils::grant_gold(&mut app, 10);

    order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    utils::run_ticks(&mut app, 12);
    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(utils::gold(app.world()), 10);
}

//
// ─── A paid queue across a role change ─────────────────────────────────────────
//

#[test]
fn change_refused_while_production_is_queued() {
    // The unrooted form trains nothing, and a queue entry is paid up front:
    // a change of form is refused while any production stands in the queue,
    // and goes through once the queue has emptied.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, shrine_id) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: shrine_id,
            type_name: "whelp".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    command_morph(&mut app, shrine_id, "golem");

    // Mid-training the command is dropped: nothing is queued behind the work,
    // and the whelp is built with its trainer unchanged.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::train_queue_len(app.world(), shrine), 1);
    assert!(
        !entity_def::orders(app.world(), shrine)
            .iter()
            .any(|order| matches!(order, Order::Morph { .. })),
        "the refused change never reached the queue"
    );
    utils::run_ticks(&mut app, 40);
    assert_eq!(type_name_of(&app, shrine), "shrine");
    assert_eq!(utils::count_of_type(app.world_mut(), "whelp"), 1);

    // Idle, the same command is taken.
    command_morph(&mut app, shrine_id, "golem");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(type_name_of(&app, shrine), "golem");
}

//
// ─── What holds a change back ──────────────────────────────────────────────────
//

#[test]
fn production_refused_while_form_changes() {
    // The unrooted form trains nothing: a unit queued while the shrine changes
    // would have no queue to land in, so the trainer is busy for the change.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, shrine_id) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 2);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: shrine_id,
            type_name: "whelp".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::train_queue_len(app.world(), shrine), 0);
    assert_eq!(utils::gold(app.world()), 10);

    utils::run_ticks(&mut app, 10);
    assert_eq!(type_name_of(&app, shrine), "golem");
}

#[test]
fn change_refused_while_workers_sit_in_berths() {
    // A berthed building carries workers in its footprint: a change of form
    // is refused while any of them is seated, and goes through once the last
    // one leaves. Staged on the orders roster, the only one with a building
    // workers sit in.
    let mut app = utils::orders_app();
    let (house, house_id) = utils::create_owned(&mut app, "shaft_house", 10, 10, 0);
    // Deep enough that the seam outlasts the test: the sylph draws five out of
    // it every two ticks.
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(house)
        .unwrap()
        .amount = 500;
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 10, 0);

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: house_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());

    command_morph(&mut app, house_id, "walking_shaft");
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(type_name_of(&app, house), "shaft_house");
    assert!(
        !entity_def::orders(app.world(), house)
            .iter()
            .any(|order| matches!(order, Order::Morph { .. })),
        "the refused change never reached the queue"
    );

    // The berth given up, the same command is taken.
    utils::stop_orders(app.world_mut(), sylph);
    utils::run_ticks(&mut app, 1);
    command_morph(&mut app, house_id, "walking_shaft");
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(type_name_of(&app, house), "walking_shaft");
}

#[test]
fn rooting_puts_static_footprint_back() {
    // The way back: a mover's claim comes off and the building's static
    // footprint goes on, so long-range planning sees the wall again.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (golem, _) = utils::create_owned(&mut app, "golem", 10, 10, 0);

    order_morph(&mut app, golem, "shrine");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, golem), "shrine");
    let world = app.world();
    let grid = world.resource::<Map>().nav_grid();
    for cell in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        let cell = CellPos::new(cell.0, cell.1);
        assert!(
            !grid.is_statically_passable_by(utils::GROUND, cell),
            "the shrine stands on the static plane at {cell:?}"
        );
        assert!(
            !grid.is_claimed_by(utils::GROUND, cell),
            "the golem's claim came off at {cell:?}"
        );
    }
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(expected = "a type change must not drop a paid production queue")]
fn change_dropping_paid_queue_panics() {
    // The illegal state the lifecycle prevents, staged directly: entries in
    // the queue with no Train order to flush and refund them. Landing here
    // would forfeit what the player paid for, so it is caught rather than
    // quietly swallowed.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, _) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    app.world_mut()
        .entity_mut(shrine)
        .get_mut::<TrainQueueComponent>()
        .expect("the shrine trains")
        .0
        .push_back("whelp".to_string());

    order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 15);
}

//
// ─── Where the new form stands ────────────────────────────────────────────────
//

#[test]
fn rooting_off_lattice_settles_position_onto_its_cells() {
    let mut app = utils::continuous_orders_app();
    let (shaft, shaft_id) = utils::create_owned(&mut app, "walking_shaft", 10, 10, 0);

    // Where a continuous mover really stands: part way across a cell, which is
    // legal for a claim and impossible for a footprint. Which way the
    // quantisation goes is pinned by
    // `interim_form_settles_onto_cell_mover_stood_on`; what this pins is that
    // a form which stands still does not keep the fraction at all.
    app.world_mut()
        .get_mut::<LocationComponent>(shaft)
        .unwrap()
        .position = FixedUVec2::new(utils::fixed("10.4"), utils::fixed("10.0"));

    command_morph(&mut app, shaft_id, "shaft_house");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        app.world()
            .get::<EntityInfoComponent>(shaft)
            .unwrap()
            .type_name(),
        "shaft_house",
        "it rooted"
    );

    // The cells it holds round to ten, and the position settles onto them
    // from 10.4: nothing is left drawing or measuring the building off the
    // cells it stands on.
    assert_eq!(
        entity_def::occupied_rect(app.world(), shaft).origin,
        CellPos::new(10, 10)
    );
    assert_eq!(
        entity_def::position(app.world(), shaft),
        FixedUVec2::new(utils::fixed("10.0"), utils::fixed("10.0"))
    );
}

#[test]
fn change_between_cells_keeps_position_of_form_that_moves() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    app.world_mut()
        .get_mut::<LocationComponent>(whelp)
        .unwrap()
        .position = utils::part_way("10.6", "10.3");
    utils::run_ticks(&mut app, 1);

    // Ten ticks of changing, and both forms claim a single cell, so nothing
    // recentres either.
    order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, whelp), "wisp", "it changed");

    // A claim is wherever the mover is, so the fraction it was walking on
    // survives the change.
    assert_eq!(
        utils::position_of(app.world(), whelp),
        utils::part_way("10.6", "10.3")
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The type an entity currently is.
fn type_name_of(app: &bevy::prelude::App, entity: bevy::prelude::Entity) -> String {
    app.world()
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("a live entity carries its info")
        .type_name()
        .to_string()
}

/// Commands a change into `type_name` the way a player does: selects the
/// entity and issues the morph command, so the executor judges it.
fn command_morph(app: &mut bevy::prelude::App, entity: SimulationId, type_name: &str) {
    utils::select(app, entity);
    utils::push_command(
        app,
        PlayerCommand::Morph {
            type_name: type_name.to_string(),
            flush: true,
        },
    );
}

/// Pushes a Morph order into `type_name` onto the entity's queue.
fn order_morph(app: &mut bevy::prelude::App, entity: bevy::prelude::Entity, type_name: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: type_name.to_string(),
            },
            None,
        );
}
