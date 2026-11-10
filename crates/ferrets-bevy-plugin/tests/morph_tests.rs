//! Form changes on the engine's own content: the grid swaps, the pools, and
//! the payment, staged on synthetic types so the mechanics are pinned apart
//! from any game's balance.

mod utils;

use bevy::prelude::{App, Mut};
use ferrets_bevy_plugin::map;
use ferrets_content::{
    attack::Slain,
    cost::Cost,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry,
    },
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    price,
    quantity::Quantity,
    registry::ContentRegistry,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::FixedU64;
use ferrets_nav_grid::{layer_mask::LayerMask, nav_grid::NavGrid};
use ferrets_simulation::{
    beside::Lean,
    command::PlayerCommand,
    components::{
        entity_info::EntityInfoComponent,
        last_hit::{self, LastHitComponent},
        location::LocationComponent,
        morph::MorphComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        pools::PoolsComponent,
        rally::RallyTarget,
        resource::ResourceSourceComponent,
        train::TrainQueueComponent,
    },
    entity_def,
    game_loop::damage,
    ground,
    map::Map,
    movement_model::MovementModel,
    occupancy::OccupancyClass,
    order::Order,
    rally,
    session::{player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

//
// ─── The grid swap ─────────────────────────────────────────────────────────────
//

#[test]
fn same_layer_growth_lands_under_continuous_model() {
    // Growing 1x1 -> 3x3 on the same layer recenters the anchor onto ground
    // the whelp's own claim covers: the change must lift that claim out of
    // the destination's way.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    utils::order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn same_layer_growth_lands_under_cell_model() {
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    utils::order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn growth_asks_idle_ally_off_its_ground_and_waits_for_it() {
    // The giant the whelp grows into takes (9..11, 9..11); an idle whelp of the
    // same side stands on (11, 11). The change does not start over it: it asks
    // it off, waits until it has walked clear, and only then begins.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (bystander, _) = utils::create_owned(&mut app, "whelp", 11, 11, 0);

    // Asked off on the first tick, the bystander crosses to (12, 11), the
    // nearest cell clear of the giant; the change begins the next tick, once
    // the crossing has left the ground, and its ten ticks land it on the
    // twelfth.
    utils::order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, whelp), "whelp");
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, whelp), "giant");
    assert_eq!(
        utils::anchor_of(app.world_mut(), bystander),
        CellPos::new(12, 11)
    );
}

#[test]
fn change_ordered_over_idle_ally_is_accepted_and_lands() {
    // The player orders the giant's change with an idle whelp of the same
    // side on (11, 11) of its ground: the order is accepted, the whelp asked
    // off, and the change lands on the twelfth tick after the order starts.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::create_owned(&mut app, "whelp", 11, 11, 0);
    utils::select(&mut app, whelp_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "giant".into(),
            flush: true,
        },
    );
    // The command lands on the third tick; waiting a tick on the bystander,
    // the change begins on the fourth and its ten ticks land it on the
    // fourteenth.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(!utils::order_queue_is_empty(app.world_mut(), whelp));
    utils::run_ticks(&mut app, 10);
    assert_eq!(type_name_of(&app, whelp), "whelp");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, whelp), "giant");
}

#[test]
fn change_into_mover_form_asks_off_only_bodies_its_claims_meet() {
    // Under the continuous model the whelp grows into a giant on
    // (9..11, 9..11), a body on the claim plane. One idle whelp rests on
    // (11, 11), its claim on the ground; another with its middle at
    // (12.2, 10.5) holds (12, 10), its body reaching 0.3 onto it. A
    // placement on the claim plane measures bodies by their claims: the first
    // is asked off on the first tick, the second never, and stays where it
    // rests.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (inside, _) = utils::create_owned(&mut app, "whelp", 11, 11, 0);
    let (lying, _) = utils::create_owned(&mut app, "whelp", 14, 10, 0);
    utils::walk_onto(&mut app, lying, utils::point("12.2", "10.5"), 20);

    utils::order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::move_target(&app, inside), Some(utils::pos(12, 11)));
    assert_eq!(utils::move_target(&app, lying), None);
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        utils::position_of(app.world(), lying),
        utils::point("12.2", "10.5")
    );
}

#[test]
fn growth_waiting_on_its_ground_is_paid_only_once_it_begins() {
    // The brute's change costs 10 gold; an idle whelp of the same side stands
    // on (11, 11) of the ground it reserves.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::create_owned(&mut app, "whelp", 11, 11, 0);
    utils::grant_gold(&mut app, 10);

    // The bystander is asked off on the first tick and the change waits,
    // nothing paid; its crossing leaves the ground and the change begins on
    // the second, paying then: 10 − 10.
    utils::order_morph(&mut app, whelp, "brute");
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        app.world()
            .get::<MorphComponent>(whelp)
            .map(MorphComponent::is_under_way),
        Some(false)
    );
    assert_eq!(utils::gold(app.world()), 10);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        app.world()
            .get::<MorphComponent>(whelp)
            .map(MorphComponent::is_under_way),
        Some(true)
    );
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn growth_its_owner_cannot_pay_asks_nobody_off_its_ground() {
    // The brute's change costs 10 gold, and the player has none; an idle
    // whelp of the same side stands on (11, 11) of the ground it reserves.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (bystander, _) = utils::create_owned(&mut app, "whelp", 11, 11, 0);

    // The change is refused on the first tick, before anyone is asked off.
    utils::order_morph(&mut app, whelp, "brute");
    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, whelp));
    assert!(world.get::<MorphComponent>(whelp).is_none());
    assert!(utils::order_queue_is_empty(world, bystander));
    assert_eq!(utils::anchor_of(world, bystander), CellPos::new(11, 11));
}

#[test]
fn canceled_growth_still_waiting_on_its_ground_leaves_nothing_behind() {
    // Walled in as below, the bystander on (11, 11) has nowhere to go, so the
    // brute's change waits on it; the player calls it off mid-wait.
    let mut app = utils::morph_app(MovementModel::Cell);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 9..=11 {
            for y in 9..=11 {
                map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
            }
        }
    }
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::create_owned(&mut app, "whelp", 11, 11, 0);
    utils::grant_gold(&mut app, 10);

    utils::order_morph(&mut app, whelp, "brute");
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        app.world()
            .get::<MorphComponent>(whelp)
            .map(MorphComponent::is_under_way),
        Some(false),
        "the change waits on its ground"
    );
    utils::push_command(&mut app, PlayerCommand::CancelMorph { entity: whelp_id });
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(utils::order_queue_is_empty(app.world_mut(), whelp));
    assert!(app.world().get::<MorphComponent>(whelp).is_none());
    assert_eq!(type_name_of(&app, whelp), "whelp");
    // Nothing was paid while it waited, so nothing comes back.
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn growth_gives_up_on_ally_with_nowhere_to_go() {
    // Every cell off the giant's (9..11, 9..11) is walled: the idle whelp on
    // (11, 11) has nowhere to walk to, and the change waits for it, then gives
    // up without having begun.
    let mut app = utils::morph_app(MovementModel::Cell);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 9..=11 {
            for y in 9..=11 {
                map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
            }
        }
    }
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::create_owned(&mut app, "whelp", 11, 11, 0);

    // Judged once a tick from the tick the order starts, as a founding is:
    // forty judgments of patience, and the forty-first, on the forty-first
    // tick, gives the order up.
    utils::order_morph(&mut app, whelp, "giant");
    utils::run_ticks(&mut app, 40);
    assert!(!utils::order_queue_is_empty(app.world_mut(), whelp));
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), whelp));
    assert_eq!(type_name_of(&app, whelp), "whelp");
}

#[test]
fn odd_growth_settles_on_lattice_under_cell_model() {
    // 1x1 -> 2x2 keeps the middle at 10.5, its low corner at 10.5 − 1 = 9.5;
    // the cell model keeps every body on the lattice, so the ogre settles
    // anchored at 10, the cell 9.5 rounds to, its middle at 10 + 1 = 11, and
    // holds cells 10..=11 — lined up on them, free to move on.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);

    utils::order_morph(&mut app, whelp, "ogre");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "ogre");
    assert_eq!(
        utils::position_of(app.world(), whelp),
        utils::point("11", "11")
    );
    utils::select(&mut app, whelp_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(15, 15),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_ne!(
        utils::position_of(app.world(), whelp),
        utils::point("11", "11")
    );
}

#[test]
fn odd_growth_keeps_middle_under_continuous_model() {
    // Continuous bodies rest anywhere: the ogre's middle stays exactly where
    // the whelp's was, 10.5.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::order_morph(&mut app, whelp, "ogre");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "ogre");
    assert_eq!(
        utils::position_of(app.world(), whelp),
        utils::point("10.5", "10.5")
    );
}

#[test]
fn growth_at_origin_settles_on_map() {
    // A whelp on cell (0, 4) has its middle at (0.5, 4.5); a 3×3 giant there
    // would reach 0.5 − 3/2 before the origin. Held at x 3/2 = 1.5, the
    // giant is anchored at (0, 3) — low edges 1.5 − 1.5 = 0 and
    // 4.5 − 1.5 = 3 — with its middle at (1.5, 4.5), under either model.
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        let mut app = utils::morph_app(model);
        let (whelp, _) = utils::create_owned(&mut app, "whelp", 0, 4, 0);
        utils::run_ticks(&mut app, 1);

        utils::order_morph(&mut app, whelp, "giant");
        utils::run_ticks(&mut app, 15);

        assert_eq!(type_name_of(&app, whelp), "giant", "{model:?}");
        assert_eq!(
            utils::anchor_of(app.world(), whelp),
            CellPos::new(0, 3),
            "{model:?}"
        );
        assert_eq!(
            utils::position_of(app.world(), whelp),
            utils::point("1.5", "4.5"),
            "{model:?}"
        );
    }
}

#[test]
fn rooting_back_and_forth_keeps_anchor() {
    // A 1×1 sapling on (5, 5) has its middle at (5.5, 5.5). Rooted into a
    // 2×2 grove, its low edges 5.5 − 1 = 4.5 lie halfway between 4 and 5:
    // the tie goes to 5, its old anchor, the grove covering cells 5–6 with
    // its middle at 5 + 1 = 6. Uprooted, the low edges 6 − 1/2 = 5.5 lie
    // halfway between 5 and 6: the tie goes to 5, the grove's anchor, and
    // the cell-model sapling stands on (5, 5) again. A continuous sapling
    // rests anywhere and keeps the grove's middle (6, 6), from which the
    // next grove's low edges 6 − 1 = 5 are no tie. Neither drifts, ten
    // times over.
    for (model, sapling_rests) in [
        (MovementModel::Cell, utils::point("5.5", "5.5")),
        (MovementModel::Continuous, utils::point("6", "6")),
    ] {
        let mut app = rooting_app(model);
        let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
        utils::run_ticks(&mut app, 1);

        for round in 0..10 {
            utils::order_morph(&mut app, sapling, "grove");
            utils::run_ticks(&mut app, 5);
            assert_eq!(type_name_of(&app, sapling), "grove", "{model:?} {round}");
            assert_eq!(
                utils::position_of(app.world(), sapling),
                utils::point("6", "6"),
                "{model:?} {round}"
            );

            utils::order_morph(&mut app, sapling, "sapling");
            utils::run_ticks(&mut app, 5);
            assert_eq!(type_name_of(&app, sapling), "sapling", "{model:?} {round}");
            assert_eq!(
                utils::position_of(app.world(), sapling),
                sapling_rests,
                "{model:?} {round}"
            );
        }
    }
}

#[test]
fn unrooting_swaps_static_footprint_for_claim() {
    // Same layers occupied, same size — only the plane changes: the shrine's
    // static footprint must come off the grid and the golem's claim go on,
    // or the ghost of the building walls its own unit in forever.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (shrine, _) = utils::create_owned(&mut app, "shrine", 10, 10, 0);

    utils::order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, shrine), "golem");
    let world = app.world();
    let grid = world.resource::<Map>().nav_grid();
    for cell in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        let cell = CellPos::new(cell.0, cell.1);
        assert!(
            !grid.is_statically_occupied(utils::GROUND, cell),
            "the shrine's static footprint survived its unrooting at {cell:?}"
        );
        assert!(
            grid.is_claimed(utils::GROUND, cell),
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

    utils::order_morph(&mut app, whelp, "husk");
    utils::run_ticks(&mut app, 3);

    assert_eq!(type_name_of(&app, whelp), "husk");
    // 30 − 10 = 20 of the whelp's 30, carried onto the husk's 10 as
    // 20 × 10 / 30, in binary fixed point.
    assert_eq!(
        utils::health(&app, whelp),
        FixedU64::from_num(20) * FixedU64::from_num(10) / FixedU64::from_num(30),
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

    utils::order_morph(&mut app, whelp, "boulder");
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

    // The golem can walk, so a rally wrongly due would carry it off; a
    // change of form sends nobody, and it stands where it unrooted.
    utils::order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 11 + 5);

    assert_eq!(type_name_of(&app, shrine), "golem");
    assert!(entity_def::orders(app.world(), shrine).is_empty());
    // The 2×2 golem anchored at (10, 10): its middle at 10 + 1 = 11.
    assert_eq!(
        utils::position_of(app.world(), shrine),
        utils::point("11", "11")
    );
}

#[test]
fn form_without_pool_keeps_empty_pool_store() {
    // The whelp's one pool is its 30 health; the wisp declares none. The pool
    // goes with the stat, because a zero-maximum pool would read as dead
    // rather than poolless, and the store stays on, empty.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    assert_eq!(utils::health_as_u32(&app, whelp), 30);
    // A hit remembered on the health pool goes with it.
    last_hit::record(app.world_mut(), whelp, whelp_id, 0);

    utils::order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 15);
    assert!(
        app.world().get::<LastHitComponent>(whelp).is_none(),
        "the last hit outlived the health pool it landed on"
    );

    assert_eq!(type_name_of(&app, whelp), "wisp");
    assert!(
        app.world()
            .entity(whelp)
            .get::<PoolsComponent>()
            .expect("a simulation entity carries a pool store")
            .is_empty(),
        "a poolless form kept a pool"
    );
    assert!(
        app.world()
            .resource::<ferrets_simulation::entity_index::EntityIndex>()
            .alive(whelp_id)
            .is_some(),
        "shedding the pool must not read as dying"
    );
}

#[test]
fn form_gaining_pool_starts_it_full() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 15);
    assert_eq!(type_name_of(&app, whelp), "wisp");

    // Back into a form with health: the pool starts full — there is no old
    // proportion to carry when the old form had no pool at all.
    utils::order_morph(&mut app, whelp, "whelp");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(utils::health_as_u32(&app, whelp), 30);
}

//
// ─── Canceling ────────────────────────────────────────────────────────────────
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
                target: utils::pos(20, 10),
                size: ferrets_geometry::cell_size::CellSize::ONE,
                range: 0,
            },
            None,
        );
    utils::order_morph(&mut app, whelp, "husk");
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

    utils::order_morph(&mut app, whelp, "wyrm");

    // Paid and in its chrysalis the tick the change starts; a full whelp is
    // a full chrysalis.
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    assert_eq!(utils::gold(app.world()), 0);
    assert_eq!(utils::health_as_u32(&app, whelp), 60);

    utils::run_ticks(&mut app, 12);
    assert_eq!(type_name_of(&app, whelp), "wyrm");
}

#[test]
fn interim_form_settles_onto_cell_mover_stood_on() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    // Part way across a cell boundary: the body's low edge (10.6, 10.3) rounds
    // onto (11, 10) and floors into (10, 10).
    utils::move_body(&mut app, whelp, utils::point("11.1", "10.8"));

    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    assert_eq!(
        utils::position_of(app.world(), whelp),
        utils::pos(11, 10),
        "the chrysalis settles onto the cell it holds, not the fraction the whelp walked on"
    );
    let map = app.world().resource::<Map>();
    assert!(
        map.nav_grid()
            .is_statically_occupied(utils::GROUND, CellPos::new(11, 10)),
        "the chrysalis holds the cell the whelp stood on"
    );
    assert!(
        !map.nav_grid()
            .is_statically_occupied(utils::GROUND, CellPos::new(10, 10)),
        "and not the cell its position floored into"
    );
}

#[test]
fn canceled_change_takes_interim_form_off() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    utils::soft_cancel_orders(app.world_mut(), whelp);
    utils::run_ticks(&mut app, 1);

    // Back to a whelp with the price returned.
    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn cancel_morph_command_calls_off_change_and_pays_its_price_back() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, whelp_id) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    // 10 granted − 10 for the wyrm.
    assert_eq!(utils::gold(app.world()), 0);

    // The player's own command, rather than a helper reaching past it.
    utils::push_command(&mut app, PlayerCommand::CancelMorph { entity: whelp_id });
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(
        utils::gold(app.world()),
        10,
        "a refundable change pays back"
    );
}

#[test]
fn change_taken_away_keeps_price_spent_though_it_is_refundable() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    // 10 granted − 10 for the wyrm.
    assert_eq!(utils::gold(app.world()), 0);

    // The same refundable change as the test above, taken away rather than
    // called off: the form still reverts, and the price stays spent.
    utils::force_cancel_orders(app.world_mut(), whelp);
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, whelp), "whelp");
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn dying_entity_keeps_interim_form() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    spawn::destroy_entity(app.world_mut(), whelp);
    utils::run_ticks(&mut app, 1);

    // The canceled change does not dress the corpse as a whelp again.
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

    utils::order_morph(&mut app, whelp, "wyrm");
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
    utils::command_morph(&mut app, shrine_id, "golem");

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
    utils::command_morph(&mut app, shrine_id, "golem");
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

    utils::order_morph(&mut app, shrine, "golem");
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
    assert!(entity_def::attached(app.world(), sylph));

    utils::command_morph(&mut app, house_id, "walking_shaft");
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(type_name_of(&app, house), "shaft_house");
    assert!(
        !entity_def::orders(app.world(), house)
            .iter()
            .any(|order| matches!(order, Order::Morph { .. })),
        "the refused change never reached the queue"
    );

    // The berth given up, the same command is taken.
    utils::force_cancel_orders(app.world_mut(), sylph);
    utils::run_ticks(&mut app, 1);
    utils::command_morph(&mut app, house_id, "walking_shaft");
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(type_name_of(&app, house), "walking_shaft");
}

#[test]
fn rooting_puts_static_footprint_back() {
    // The way back: a mover's claim comes off and the building's static
    // footprint goes on, so long-range planning sees the wall again.
    let mut app = utils::morph_app(MovementModel::Cell);
    let (golem, _) = utils::create_owned(&mut app, "golem", 10, 10, 0);

    utils::order_morph(&mut app, golem, "shrine");
    utils::run_ticks(&mut app, 15);

    assert_eq!(type_name_of(&app, golem), "shrine");
    let world = app.world();
    let grid = world.resource::<Map>().nav_grid();
    for cell in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        let cell = CellPos::new(cell.0, cell.1);
        assert!(
            grid.is_statically_occupied(utils::GROUND, cell),
            "the shrine stands on the static plane at {cell:?}"
        );
        assert!(
            !grid.is_claimed(utils::GROUND, cell),
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

    utils::order_morph(&mut app, shrine, "golem");
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
        .position = utils::point("10.9", "10.5");

    utils::command_morph(&mut app, shaft_id, "shaft_house");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        app.world()
            .get::<EntityInfoComponent>(shaft)
            .unwrap()
            .type_name(),
        "shaft_house",
        "it rooted"
    );

    // The cell it holds, its low edge 10.4 rounded, is ten, and the position
    // snaps onto the cell it is anchored at from 10.9: nothing is left drawing
    // or measuring the building off the cells it stands on.
    assert_eq!(utils::anchor_of(app.world(), shaft), CellPos::new(10, 10));
    assert_eq!(entity_def::position(app.world(), shaft), utils::pos(10, 10));
}

#[test]
fn change_between_cells_keeps_position_of_form_that_moves() {
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    utils::move_body(&mut app, whelp, utils::point("11.1", "10.8"));

    // Ten ticks of changing, and both forms claim a single cell, so nothing
    // recenters either.
    utils::order_morph(&mut app, whelp, "wisp");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, whelp), "wisp", "it changed");

    // A claim is wherever the mover is, so the fraction it was walking on
    // survives the change.
    assert_eq!(
        utils::position_of(app.world(), whelp),
        utils::point("11.1", "10.8")
    );
}

//
// ─── What stands underfoot ─────────────────────────────────────────────────────
//

#[test]
fn lowered_gate_is_walked_over() {
    let mut app = underfoot_app();
    let (gate, _) = utils::create_owned(&mut app, "gate", 10, 10, 0);
    let (digger, _) = utils::create_owned(&mut app, "digger", 8, 10, 0);

    // Lowered, the gate claims none of its cells: the digger walks onto one.
    utils::order_morph(&mut app, gate, "gate_lowered");
    utils::run_ticks(&mut app, 4);
    assert_eq!(type_name_of(&app, gate), "gate_lowered");
    utils::order_move(&mut app, digger, 11, 10);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::anchor_of(app.world(), digger), CellPos::new(11, 10));
}

#[test]
fn raising_gate_asks_own_walker_off() {
    let mut app = underfoot_app();
    let (gate, _) = utils::create_owned(&mut app, "gate_lowered", 10, 10, 0);
    let (digger, _) = utils::create_owned(&mut app, "digger", 11, 10, 0);

    // The raised gate needs its four cells: the idle digger on one steps off
    // and the gate rises.
    utils::order_morph(&mut app, gate, "gate");
    utils::run_ticks(&mut app, 30);
    assert_eq!(type_name_of(&app, gate), "gate");
    // The nearest cell off the gate's 2×2 at (10, 10): (11, 9) north, its tie
    // with (12, 10) east going to the lower row.
    assert_eq!(utils::anchor_of(app.world(), digger), CellPos::new(11, 9));
}

#[test]
fn raising_gate_asks_off_walker_that_moves_before_it() {
    let mut app = underfoot_app_on(MovementModel::Continuous);
    // Created first, the digger runs its orders before the gate's in a tick:
    // by the time the gate judges its ground again, the digger has already
    // stepped off it this tick, its claim moving with it.
    let (digger, _) = utils::create_owned(&mut app, "digger", 11, 10, 0);
    let (gate, _) = utils::create_owned(&mut app, "gate_lowered", 10, 10, 0);

    utils::order_morph(&mut app, gate, "gate");
    utils::run_ticks(&mut app, 30);
    assert_eq!(type_name_of(&app, gate), "gate");
    // The nearest cell off the gate, as under the cell model.
    assert_eq!(utils::anchor_of(app.world(), digger), CellPos::new(11, 9));
}

#[test]
fn raising_gate_is_refused_while_enemy_stands_on_it() {
    let mut app = underfoot_app();
    let (gate, _) = utils::create_owned(&mut app, "gate_lowered", 10, 10, 0);
    let (digger, _) = utils::create_owned(&mut app, "digger", 11, 10, 1);

    // An enemy on the gate is not asked off: the raise is refused at once,
    // not left waiting for the ground.
    utils::order_morph(&mut app, gate, "gate");
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), gate));
    utils::run_ticks(&mut app, 60);
    assert_eq!(type_name_of(&app, gate), "gate_lowered");
    assert_eq!(utils::anchor_of(app.world(), digger), CellPos::new(11, 10));
}

#[test]
fn change_into_static_form_is_refused_over_burrowed_entity() {
    let mut app = underfoot_app();
    utils::create_owned(&mut app, "mole", 10, 10, 0);
    // The mole claims nothing on the grid, so the digger stands on its cell.
    // The den would hold that ground, and the change reserves its footprint
    // at issue: it is refused there, as a site founded on the cell is, before
    // anything is paid.
    let (digger, _) = utils::create_owned(&mut app, "digger", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    utils::order_morph(&mut app, digger, "den");
    utils::run_ticks(&mut app, 4);

    assert_eq!(type_name_of(&app, digger), "digger");
    assert_eq!(utils::gold(app.world()), 10, "nothing was paid");
    assert!(utils::order_queue_is_empty(app.world_mut(), digger));
}

#[test]
fn burrowed_form_changing_in_place_is_not_refused_by_its_own_footprint() {
    let mut app = underfoot_app();
    let (mole, _) = utils::create_owned(&mut app, "mole", 10, 10, 0);

    // Both forms stand underfoot on the one cell: the ground the vole needs is
    // what the mole itself holds, which is no reason to refuse it.
    utils::order_morph(&mut app, mole, "vole");
    utils::run_ticks(&mut app, 4);

    assert_eq!(type_name_of(&app, mole), "vole");
}

#[test]
fn burrowed_form_revalidated_in_place_is_not_refused_by_its_own_footprint() {
    let mut app = underfoot_app();
    let (mole, _) = utils::create_owned(&mut app, "mole", 10, 10, 0);

    // Judged only as it lands, the 2×2 shrew's ground takes in the cell the
    // mole itself holds, which is no reason to refuse it.
    utils::order_morph(&mut app, mole, "shrew");
    utils::run_ticks(&mut app, 4);

    assert_eq!(type_name_of(&app, mole), "shrew");
}

#[test]
fn change_into_form_of_same_marks_is_refused_over_terrain_it_does_not_take() {
    // The wader's domain takes every terrain, the dryling's land alone; both
    // claim one cell of the ground layer, so the change moves nothing on the
    // grid — but judged as it lands, the dryling may not stand on the water
    // the wader stands on, and the change fizzles there.
    let mut app = wading_app();
    let (afloat, _) = utils::create_owned(&mut app, "wader", 10, 10, 0);
    let (ashore, _) = utils::create_owned(&mut app, "wader", 4, 4, 0);

    utils::order_morph(&mut app, afloat, "dryling");
    utils::order_morph(&mut app, ashore, "dryling");
    utils::run_ticks(&mut app, 6);

    assert_eq!(type_name_of(&app, afloat), "wader");
    assert_eq!(type_name_of(&app, ashore), "dryling");
}

#[test]
fn change_into_static_form_is_refused_over_terrain_it_does_not_take() {
    let mut app = wading_app();
    let (afloat, _) = utils::create_owned(&mut app, "wader", 10, 10, 0);

    utils::order_morph(&mut app, afloat, "hut");
    utils::run_ticks(&mut app, 6);

    assert_eq!(type_name_of(&app, afloat), "wader");
}

#[test]
fn change_into_form_wider_than_map_is_refused() {
    // The map is 32 cells across: a 40×40 colossus has no place on it, and the
    // sapling keeps its form.
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Cell);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            utils::walker("sapling", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_morphs([change("colossus", MorphPlacement::Revalidate, Vec::new())]),
        );
        registry.register(
            EntityTypeDef::new("colossus")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(40, 40),
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20)),
        );
        registry.validate();
    }
    utils::start_game(&mut app);
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 10, 10, 0);

    utils::order_morph(&mut app, sapling, "colossus");
    utils::run_ticks(&mut app, 6);

    assert_eq!(type_name_of(&app, sapling), "sapling");
    assert!(!entity_def::changing(app.world(), sapling));
}

#[test]
fn change_landing_nearby_stands_only_where_new_form_steps_from_old() {
    // Mid-pool at (10,10) every cell beside the wader is water, none a settler
    // steps onto from there: the change finds no room and the wader keeps its
    // form. At the pool's edge, (10,9), the land of row 8 lies beside it, and
    // (10,8) straight out takes the settler.
    let mut app = wading_app();
    let (afloat, _) = utils::create_owned(&mut app, "wader", 10, 10, 0);
    let (at_edge, _) = utils::create_owned(&mut app, "wader", 10, 9, 0);

    utils::order_morph(&mut app, afloat, "settler");
    utils::order_morph(&mut app, at_edge, "settler");
    utils::run_ticks(&mut app, 6);

    assert_eq!(type_name_of(&app, afloat), "wader");
    assert_eq!(type_name_of(&app, at_edge), "settler");
    assert_eq!(utils::anchor_of(app.world(), at_edge), CellPos::new(10, 8));
}

#[test]
fn worker_seated_inside_its_job_steps_out_of_whole_footprint_to_change() {
    // A 3×3 core vein at (8..10, 5..7) seats one carrier at its middle cell,
    // (9, 6), every neighbor of which is the vein's own. A shifter seated
    // there, told to change, comes out of the vein's whole footprint: it
    // steps out beside the vein — of the cells two off its berth, (9, 4) is
    // first by row — and the change begins.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &[], |registry, _| {
        registry.register(utils::core_vein());
        let sylph = registry
            .entity("sylph")
            .expect("the orders content declares a sylph")
            .clone();
        let mut shifter = sylph.with_morphs([MorphTransition::new(
            "sylph",
            MorphCourse::direct(MorphInterrupted::Reverts),
            Quantity::Constant(10),
            MorphPlacement::Revalidate,
            MorphCancel::Refundable,
            MorphReason::Change,
            Vec::new(),
            Vec::new(),
            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
        )]);
        shifter.name = "shifter".to_string();
        registry.register(shifter);
    });
    let (vein, vein_id) =
        utils::create_entity(app.world_mut(), "core_vein", utils::cell(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .unwrap()
        .amount = 500;
    let (shifter, shifter_id) = utils::create_owned(&mut app, "shifter", 7, 6, 0);
    utils::select(&mut app, shifter_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: vein_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(entity_def::attached(app.world(), shifter));
    assert_eq!(utils::anchor_of(app.world(), shifter), CellPos::new(9, 6));

    utils::command_morph(&mut app, shifter_id, "sylph");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(!entity_def::attached(app.world(), shifter));
    assert!(entity_def::changing(app.world(), shifter));
    assert_eq!(utils::anchor_of(app.world(), shifter), CellPos::new(9, 4));
}

#[test]
fn canceled_change_of_berthed_worker_leaves_it_on_land_it_takes() {
    // A dry sylph, its domain taking land alone, sits in the berth of a shaft
    // house at (10,10) ringed by water but for (9,11), and is told to change
    // into a form that takes water too. It leaves the berth for the one cell
    // of land in the ring, (9,11); canceled, the dry sylph stays there.
    let water = [
        (9, 9),
        (10, 9),
        (11, 9),
        (9, 10),
        (11, 10),
        (10, 11),
        (11, 11),
    ];
    let mut app = utils::dry_orders_app(MovementModel::Cell, &water, |registry, dry| {
        let sylph = registry
            .entity("sylph")
            .expect("the orders content declares a sylph")
            .clone();
        let mut dry_sylph =
            utils::in_domain(sylph.clone(), dry).with_morphs([MorphTransition::new(
                "wet_sylph",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(10),
                MorphPlacement::Revalidate,
                MorphCancel::Refundable,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )]);
        dry_sylph.name = "dry_sylph".to_string();
        registry.register(dry_sylph);
        let mut wet_sylph = sylph;
        wet_sylph.name = "wet_sylph".to_string();
        registry.register(wet_sylph);
    });
    let (house, house_id) = utils::create_owned(&mut app, "shaft_house", 10, 10, 0);
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(house)
        .unwrap()
        .amount = 500;
    let (sylph, sylph_id) = utils::create_owned(&mut app, "dry_sylph", 8, 12, 0);
    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: house_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(entity_def::attached(app.world(), sylph));

    utils::command_morph(&mut app, sylph_id, "wet_sylph");
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!entity_def::attached(app.world(), sylph));
    assert_eq!(utils::anchor_of(app.world(), sylph), CellPos::new(9, 11));

    utils::soft_cancel_orders(app.world_mut(), sylph);
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, sylph), "dry_sylph");
    assert_eq!(utils::anchor_of(app.world(), sylph), CellPos::new(9, 11));
}

//
// ─── Reserved ground ──────────────────────────────────────────────────────────
//

#[test]
fn reserving_change_claims_layers_its_standing_claim_lacks_under_it() {
    // A 1×1 ground keeper changes into a 2×2 titan on ground and air: on its
    // own cell the reservation claims air alone, its ground claim already
    // holding the ground; the other three cells are claimed on both.
    let mut app = titan_app();
    let (keeper, keeper_id) = utils::create_owned(&mut app, "keeper", 10, 10, 0);
    let own = utils::anchor_of(app.world(), keeper);

    utils::order_morph(&mut app, keeper, "titan");
    utils::run_ticks(&mut app, 1);

    let morph = app
        .world()
        .get::<MorphComponent>(keeper)
        .expect("the change is under way");
    assert!(
        morph
            .as_under_way()
            .and_then(|under_way| under_way.reservation.as_ref())
            .is_some(),
        "the change reserves its ground"
    );
    let grid = app.world().resource::<Map>().nav_grid();
    // One claim per layer on its own cell, both the keeper's: its standing
    // one on ground, the reservation's on air.
    assert_eq!(grid.claim_holders(utils::GROUND, own), [keeper_id.0]);
    assert_eq!(grid.claim_holders(utils::AIR, own), [keeper_id.0]);
    // The 2×2 lies within a cell of its own: 4 cells keeper-held on each
    // layer there — on ground its own cell plus the three reserved, on air
    // the four reserved.
    let around = CellRect::new(CellPos::new(own.x - 1, own.y - 1), CellSize::new(3, 3));
    for layer in [utils::GROUND, utils::AIR] {
        assert_eq!(
            around
                .cells()
                .filter(|&cell| grid.claim_holders(layer, cell) == [keeper_id.0])
                .count(),
            4
        );
    }
}

#[test]
fn placement_on_ground_reserved_for_change_is_refused() {
    // The keeper's change into a 2×2 titan holds ground ahead around its own
    // cell. A keeper-sized body placed on a reserved cell its body does not
    // lie over is refused, the cell neither claimed by a standing body nor
    // covered by one; a cell clear of the reservation takes it.
    let mut app = titan_app();
    let (keeper, keeper_id) = utils::create_owned(&mut app, "keeper", 10, 10, 0);
    let own = utils::anchor_of(app.world(), keeper);
    utils::order_morph(&mut app, keeper, "titan");
    utils::run_ticks(&mut app, 1);

    let world = app.world();
    let grid = world.resource::<Map>().nav_grid();
    let reserved = CellRect::new(CellPos::new(own.x - 1, own.y - 1), CellSize::new(3, 3))
        .cells()
        .find(|&cell| cell != own && grid.claim_holders(utils::GROUND, cell) == [keeper_id.0])
        .expect("the change reserves cells around the keeper's own");
    let keeper_location = world
        .resource::<ContentRegistry>()
        .entity("keeper")
        .and_then(|def| def.location)
        .expect("the keeper stands somewhere");
    let placed_on = |cell: CellPos| {
        ground::can_place(
            world,
            cell,
            keeper_location,
            OccupancyClass::Claim,
            &[],
            &[],
        )
    };

    assert!(!placed_on(reserved));
    assert!(placed_on(CellPos::new(own.x + 4, own.y)));
}

#[test]
fn building_reserving_change_keeps_ground_only_by_its_reservation() {
    // Under the continuous model a shrine on (10..11, 10..11) changes into a
    // golem and reserves its ground on the claim plane, a standing body
    // holding claims. A shrine judged beside it on (12..13, 10..11) meets
    // those claims in the ring it reads but none of the reserved cells, and
    // the ground takes it; the reserving shrine is measured by nothing but
    // its reservation.
    let mut app = utils::morph_app(MovementModel::Continuous);
    let (shrine, _) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    utils::order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::reserving(app.world(), shrine));

    let world = app.world();
    let shrine_location = world
        .resource::<ContentRegistry>()
        .entity("shrine")
        .and_then(|def| def.location)
        .expect("the shrine stands somewhere");
    assert!(ground::can_place(
        world,
        utils::cell(12, 10),
        shrine_location,
        OccupancyClass::Static,
        &[],
        &[],
    ));
}

#[test]
fn body_changing_on_reserved_ground_stands_against_contact() {
    let mut app = titan_app();
    let (keeper, _) = utils::create_owned(&mut app, "keeper", 10, 10, 0);
    let (shover, _) = utils::create_owned(&mut app, "whelp", 14, 10, 0);
    utils::order_morph(&mut app, keeper, "titan");
    utils::run_ticks(&mut app, 1);

    // A whelp pressed into the keeper's body: the contact pushes the whelp
    // off, and the keeper stands where its change reserved its ground.
    utils::move_body(&mut app, shover, utils::point("10.9", "10.5"));
    utils::run_ticks(&mut app, 1);

    // Two radii of 0.5, 0.4 apart, overlap by 0.5 + 0.5 - 0.4 = 0.6; at equal
    // weight each body's share is half, 0.3: the whelp's moves it on to
    // 10.9 + 0.3 = 11.2, the keeper's is dropped. The whelp takes only its
    // own share until a standing body is a wall to it (plan Q105), when it
    // would take all 0.6, to 11.5.
    assert_eq!(utils::position_of(app.world(), keeper), utils::pos(10, 10));
    assert_eq!(
        utils::position_of(app.world(), shover),
        utils::point("11.2", "10.5")
    );
}

//
// ─── A capture ────────────────────────────────────────────────────────────────
//

#[test]
fn change_under_way_goes_on_for_its_captor() {
    // The whelp wears the chrysalis three ticks into its ten-tick change
    // into the wyrm when player 1 takes it: the change goes on, and lands as
    // player 1's wyrm.
    let mut app = two_sided_morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "whelp", 20, 20, 1);
    utils::grant_gold(&mut app, 10);
    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");

    spawn::change_owner(app.world_mut(), whelp, 1, taker_id, Lean::Nothing);
    assert_eq!(entity_def::owner(app.world(), whelp), Some(1));
    assert!(entity_def::changing(app.world(), whelp));

    // Ten ticks of change, three done before the capture: six more leave it
    // a chrysalis, the seventh lands it.
    utils::run_ticks(&mut app, 6);
    assert_eq!(type_name_of(&app, whelp), "chrysalis");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, whelp), "wyrm");
    assert_eq!(entity_def::owner(app.world(), whelp), Some(1));
}

#[test]
fn change_under_way_taken_drops_rally_its_old_owner_set() {
    let mut app = two_sided_morph_app(MovementModel::Cell);
    let (shrine, shrine_id) = utils::create_owned(&mut app, "shrine", 10, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "whelp", 20, 20, 1);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: shrine_id,
            target: Some(RallyTarget::Position(utils::pos(20, 10))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::order_morph(&mut app, shrine, "golem");
    utils::run_ticks(&mut app, 3);
    assert!(entity_def::changing(app.world(), shrine));

    spawn::change_owner(app.world_mut(), shrine, 1, taker_id, Lean::Nothing);

    assert_eq!(rally::target_of(app.world(), shrine), None);
}

#[test]
fn body_dying_mid_change_is_not_captured_and_dies() {
    // Struck down three ticks into its change into the wyrm, the chrysalis
    // is dying when player 1 would take it: it stays player 0's, and its
    // two-tick death takes it off the map.
    let mut app = two_sided_morph_app(MovementModel::Continuous);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "whelp", 20, 20, 1);
    utils::grant_gold(&mut app, 10);
    utils::order_morph(&mut app, whelp, "wyrm");
    utils::run_ticks(&mut app, 3);
    damage::apply(
        app.world_mut(),
        taker_id,
        whelp,
        FixedU64::from_num(1000),
        Slain::Nothing,
    );

    spawn::change_owner(app.world_mut(), whelp, 1, taker_id, Lean::Nothing);
    assert_eq!(entity_def::owner(app.world(), whelp), Some(0));

    utils::run_ticks(&mut app, 3);
    utils::assert_despawned(app.world_mut(), whelp);
}

#[test]
fn dying_body_is_not_captured() {
    // Struck down first, the whelp is dying when player 1 would take it: a
    // force drop leaves its death running, and what died under player 0 is
    // not handed over.
    let mut app = two_sided_morph_app(MovementModel::Cell);
    let (whelp, _) = utils::create_owned(&mut app, "whelp", 10, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "whelp", 20, 20, 1);
    damage::apply(
        app.world_mut(),
        taker_id,
        whelp,
        FixedU64::from_num(1000),
        Slain::Nothing,
    );

    spawn::change_owner(app.world_mut(), whelp, 1, taker_id, Lean::Nothing);

    assert_eq!(entity_def::owner(app.world(), whelp), Some(0));
    assert!(entity_def::dying(app.world(), whelp));
    assert!(matches!(
        entity_def::orders(app.world(), whelp).as_slice(),
        [Order::Die]
    ));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Two humans on a cell-model map with a `mole`, a `vole` and a 2×2 `shrew`
/// that stand underfoot, the mole changing in place into the vole, reserving
/// its ground, and into the shrew, judged as it lands; a `digger` that
/// changes into a static `den` on its own cell for 10 gold, reserving the
/// ground; and a static 2×2 `gate` that lowers into an underfoot `gate_lowered`
/// and raises back, reserving its ground. Session started.
fn underfoot_app() -> App {
    underfoot_app_on(MovementModel::Cell)
}

/// [`underfoot_app`] on a map of `model`.
fn underfoot_app_on(model: MovementModel) -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    utils::install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("mole")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Underfoot,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20))
                .with_morphs([
                    change("vole", MorphPlacement::Reserve, Vec::new()),
                    change("shrew", MorphPlacement::Revalidate, Vec::new()),
                ]),
        );
        registry.register(
            EntityTypeDef::new("vole")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Underfoot,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("shrew")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    Solidity::Underfoot,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20)),
        );
        registry.register(
            utils::walker("digger", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_morphs([change(
                    "den",
                    MorphPlacement::Reserve,
                    vec![Cost::Resources(price::from([("gold", 10)]))],
                )]),
        );
        registry.register(
            EntityTypeDef::new("den")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_dying(2, []),
        );
        let gate = |name: &str, solidity, into: &str, placement| {
            EntityTypeDef::new(name)
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    solidity,
                    utils::GROUND,
                )
                .with_pool(Pool::health(50))
                .with_morphs([change(into, placement, Vec::new())])
        };
        registry.register(gate(
            "gate",
            Solidity::Solid,
            "gate_lowered",
            MorphPlacement::Revalidate,
        ));
        registry.register(gate(
            "gate_lowered",
            Solidity::Underfoot,
            "gate",
            MorphPlacement::Reserve,
        ));
        registry.validate();
    }
    utils::start_game(&mut app);
    app
}

/// One human on a map of `model` with a 1×1 `sapling` walker that roots in
/// place into a static 2×2 `grove`, and the grove that uproots back into the
/// sapling, both judged as they land. Session started.
fn rooting_app(model: MovementModel) -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    utils::install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            utils::walker("sapling", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_morphs([change("grove", MorphPlacement::Revalidate, Vec::new())]),
        );
        registry.register(
            EntityTypeDef::new("grove")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20))
                .with_morphs([change("sapling", MorphPlacement::Revalidate, Vec::new())]),
        );
        registry.validate();
    }
    utils::start_game(&mut app);
    app
}

/// A two-tick refundable change of form into `into`, its ground secured per
/// `placement`, drawing `costs` when it starts.
fn change(into: &str, placement: MorphPlacement, costs: Vec<Cost>) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::direct(MorphInterrupted::Reverts),
        Quantity::Constant(2),
        placement,
        MorphCancel::Refundable,
        MorphReason::Change,
        costs,
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}

/// [`utils::morph_app`] on a continuous-model map with the air layer over
/// the ground, with a `keeper` — the whelp — that changes in ten ticks,
/// reserving its ground, into a standing 2×2 `titan` on ground and air.
fn titan_app() -> App {
    let mut app = utils::morph_app(MovementModel::Continuous);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        utils::register_air(&mut registry);
        let mut keeper = registry
            .entity("whelp")
            .expect("the morph content declares a whelp")
            .clone()
            .with_morphs([MorphTransition::new(
                "titan",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(10),
                MorphPlacement::Reserve,
                MorphCancel::Refundable,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )]);
        keeper.name = "keeper".to_string();
        registry.register(keeper);
        registry.register(
            EntityTypeDef::new("titan")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    Solidity::Solid,
                    LayerMask::from(utils::GROUND) | utils::AIR,
                )
                .with_pool(Pool::health(60)),
        );
    }
    let mut grid = NavGrid::new(32, 32, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    grid.add_flat_layer(utils::AIR, 2);
    map::install_map(
        app.world_mut(),
        Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Continuous,
            grid,
            vec![],
        ),
    );
    app.world_mut().resource_scope(|world, mut map: Mut<Map>| {
        map.serve_movers(world.resource::<ContentRegistry>());
    });
    app
}

/// The type an entity currently is.
fn type_name_of(app: &bevy::prelude::App, entity: bevy::prelude::Entity) -> String {
    app.world()
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("a live entity carries its info")
        .type_name()
        .to_string()
}

/// [`utils::dry_orders_app`] with a `wader` — the orders soldier, its domain
/// taking every terrain — standing in a 3×3 pool of water at (9,9)..(11,11),
/// that changes into forms whose domain takes dry land alone: a `dryling`,
/// the landlubber judged as it lands; a `settler`, the same set down nearby;
/// and a static `hut`, judged as it lands.
fn wading_app() -> App {
    utils::dry_orders_app(
        MovementModel::Cell,
        &utils::water_pool(),
        |registry, dry| {
            let soldier = registry
                .entity("soldier")
                .expect("the orders content declares a soldier")
                .clone();
            let mut wader = soldier.clone().with_morphs([
                change("dryling", MorphPlacement::Revalidate, Vec::new()),
                change("settler", MorphPlacement::Nearby, Vec::new()),
                change("hut", MorphPlacement::Revalidate, Vec::new()),
            ]);
            wader.name = "wader".to_string();
            registry.register(wader);
            for name in ["dryling", "settler"] {
                let mut form = utils::in_domain(soldier.clone(), dry);
                form.name = name.to_string();
                registry.register(form);
            }
            registry.register(
                EntityTypeDef::new("hut")
                    .with_location(dry, CellSize::ONE, Solidity::Solid, utils::GROUND)
                    .with_pool(Pool::health(30)),
            );
        },
    )
}

/// [`utils::morph_app`] on `model`, seated for two humans, so a body can
/// change hands.
fn two_sided_morph_app(model: MovementModel) -> App {
    utils::morph_app_for(
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(1, PlayerType::Human, None, None),
        ],
        model,
    )
}
