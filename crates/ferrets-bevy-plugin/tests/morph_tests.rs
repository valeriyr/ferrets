//! Form changes on the engine's own content: the grid swaps, the pools, and
//! the payment, staged on synthetic types so the mechanics are pinned apart
//! from any game's balance.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        train::TrainQueueComponent,
    },
    map::Map,
    movement_model::MovementModel,
    order::Order,
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
    let (whelp, _) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);
    // One tick so the rebuilt claim plane holds the whelp's footprint.
    utils::run_ticks(&mut app, 1);

    order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn same_layer_growth_lands_under_cell_model() {
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);

    order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn unrooting_swaps_static_footprint_for_claim() {
    // Same occupation, same size — only the plane changes: the shrine's
    // static footprint must come off the grid and the golem's claim go on,
    // or the ghost of the building walls its own unit in forever.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, _) = utils::spawn_owned(&mut app, "shrine", 10, 10, 0);

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
    let (whelp, _) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);

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
fn form_without_pool_sheds_pool_component() {
    // The wisp declares no health: the pool component goes with the stat,
    // because a zero-maximum pool would read as dead rather than poolless.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, whelp_id) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);

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
    let (whelp, _) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);
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
    let (whelp, _) = utils::spawn_owned(&mut app, "whelp", 10, 10, 0);

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
// ─── A paid queue across a role change ─────────────────────────────────────────
//

#[test]
fn change_waits_for_production_it_would_cancel() {
    // The unrooted form trains nothing, and a queue entry is paid up front:
    // the change must not run off with unbuilt units. The order lifecycle is
    // what guarantees it — a soft flush leaves the Train order working, so
    // the change waits its turn and inherits an empty queue.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, shrine_id) = utils::spawn_owned(&mut app, "shrine", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: shrine_id,
            type_name: "whelp".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    order_morph(&mut app, shrine, "golem");

    // Mid-training: the change is queued behind the work, not through it.
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, shrine), "shrine");

    utils::run_ticks(&mut app, 40);
    assert_eq!(type_name_of(&app, shrine), "golem");
    assert_eq!(
        utils::count_of_type(app.world_mut(), "whelp"),
        1,
        "the paid unit was built before its trainer changed form"
    );
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
    let (shrine, _) = utils::spawn_owned(&mut app, "shrine", 10, 10, 0);
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
