//! Levels: a plateau climbed by its ramp, cliffs that stop all but cliff walkers, footprints across level edges, reach across them, and bodies set down and changing form on them.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::map;
use ferrets_content::{
    affiliation::Affiliation,
    berths::BerthGroup,
    brood::OrphanFate,
    detection::Detection,
    domain::{DomainDef, DomainId},
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldCoverage, FieldDecay, FieldDef, FieldGrowth, FieldPlacement, FieldSourceDef,
        FieldVision,
    },
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    terrains::Terrains,
    work::{Attachment, BerthStance},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_pathfinder::{
    cliff_crossing::CliffCrossing,
    layer_id::LayerId,
    layer_level::LayerLevel,
    slope::{Direction, Slope},
};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        location::LocationComponent, morph::MorphComponent, order_queue::OrderQueueComponent,
        stance::Stance,
    },
    entity_def,
    entity_index::EntityIndex,
    events::DeathCause,
    game_loop::orders::{self, Refusal},
    map::Map,
    map_data::MapData,
    movement_model::MovementModel,
    order::Order,
    session::{player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

//
// ─── Walking ──────────────────────────────────────────────────────────────────
//

#[test]
fn walker_climbs_plateau_only_by_its_ramp() {
    for (model, ramps) in [
        (MovementModel::Cell, Ramps::Short),
        (MovementModel::Continuous, Ramps::Short),
        (MovementModel::Cell, Ramps::Long),
        (MovementModel::Continuous, Ramps::Long),
    ] {
        let mut app = hills_app(model, ramps, Rise::One);
        let (walker, _) = utils::create_owned(&mut app, "walker", 2, 2, 0);
        utils::order_move(&mut app, walker, 12, 2);

        // Wherever it steps from column 7 onto the plateau's column 8, it is
        // on the ramp's rows, 6 and 7. It arrives on tick 22: eleven steps of
        // two ticks at half a cell a tick, four diagonals down to row 6, three
        // east over the ramp, and four back up to row 2: 11 × 2 = 22.
        let mut last = utils::cell_of(app.world(), walker);
        for _ in 0..22 {
            utils::run_ticks(&mut app, 1);
            let cell = utils::cell_of(app.world(), walker);
            if last.x == 7 && cell.x == 8 {
                assert!(
                    (6..=7).contains(&last.y),
                    "{model:?} {ramps:?}: it climbed the cliff from {last:?} to {cell:?}"
                );
            }
            last = cell;
        }
        assert_eq!(
            utils::position_of(app.world(), walker),
            utils::pos(12, 2),
            "{model:?} {ramps:?}"
        );
    }
}

#[test]
fn walker_never_reaches_plateau_without_ramp() {
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (walker, _) = utils::create_owned(&mut app, "walker", 2, 2, 0);
    utils::order_move(&mut app, walker, 12, 2);

    // Column 7 is the cliff's low edge, off a walker's ground. The walk ends
    // at the cliff's foot on tick 8, four steps of two ticks, and it stays.
    for _ in 0..20 {
        utils::run_ticks(&mut app, 1);
        assert!(utils::cell_of(app.world(), walker).x <= 6);
    }
    assert_eq!(utils::position_of(app.world(), walker), utils::pos(6, 0));
}

#[test]
fn cliff_walker_climbs_straight_up() {
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        let mut app = hills_app(model, Ramps::Short, Rise::One);
        let (strider, _) = utils::create_owned(&mut app, "strider", 2, 2, 0);
        utils::order_move(&mut app, strider, 12, 2);

        // Ten cells east at half a cell a tick under either model: it
        // arrives on tick 10 × 2 = 20.
        for _ in 0..20 {
            utils::run_ticks(&mut app, 1);
            assert_eq!(
                utils::cell_of(app.world(), strider).y,
                2,
                "{model:?}: it keeps to row 2"
            );
        }
        assert_eq!(
            utils::position_of(app.world(), strider),
            utils::pos(12, 2),
            "{model:?}"
        );
    }
}

#[test]
fn cliff_walker_stops_at_cliff_two_levels_tall() {
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        let mut app = hills_app(model, Ramps::None, Rise::Two);
        let (strider, _) = utils::create_owned(&mut app, "strider", 2, 2, 0);
        utils::order_move(&mut app, strider, 12, 2);

        // It walks onto the cliff's low edge at column 7 and never over it.
        // The plateau is out of its reach, so its walk ends at the nearest
        // cell it reaches: column 7's rows 0 to 7 all lie max(5, dy) = 5 from
        // (12, 2), and the first by cell order is (7, 0).
        for _ in 0..30 {
            utils::run_ticks(&mut app, 1);
            assert!(utils::cell_of(app.world(), strider).x <= 7, "{model:?}");
        }
        assert_eq!(
            utils::position_of(app.world(), strider),
            utils::pos(7, 0),
            "{model:?}"
        );
    }
}

//
// ─── Standing ─────────────────────────────────────────────────────────────────
//

#[test]
fn only_cliff_walker_stands_on_cliff() {
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);

    // (7, 2) is the cliff's low edge, (8, 2) its high edge.
    for x in [7, 8] {
        assert!(
            utils::create_entity(app.world_mut(), "walker", utils::pos(x, 2), Some(0)).is_none()
        );
        assert!(
            utils::create_entity(app.world_mut(), "strider", utils::pos(x, 2), Some(0)).is_some()
        );
    }
}

#[test]
fn big_cliff_walker_stands_across_cliff_edge() {
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);

    // Anchored at (7, 2), a 2×2 spans low ground at column 7 and the
    // plateau at column 8.
    assert!(
        utils::create_entity(app.world_mut(), "big_walker", utils::pos(7, 2), Some(0)).is_none()
    );
    assert!(
        utils::create_entity(app.world_mut(), "big_strider", utils::pos(7, 2), Some(0)).is_some()
    );
}

#[test]
fn static_body_of_flat_layer_stands_over_slopes() {
    // The beacon's layer is at one level everywhere, so the ground layer's
    // ramp and cliff under it are no business of its.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    for (x, y) in [(7, 6), (7, 2), (8, 2)] {
        assert!(
            utils::create_entity(app.world_mut(), "beacon", utils::pos(x, y), Some(0)).is_some(),
            "a beacon stands over ({x}, {y})"
        );
    }
}

#[test]
fn static_footprint_stands_only_on_level_ground() {
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let depot = |app: &mut App, x, y| {
        utils::create_entity(app.world_mut(), "depot", utils::pos(x, y), Some(0)).is_some()
    };

    // Over the ramp's low half at (7, 6) and (7, 7), though all four cells
    // are at 1; on the plateau; on low ground. (Across the cliff's columns 7
    // and 8 a depot is refused before its levels are read: cliff cells are
    // off a walker's ground.)
    assert!(!depot(&mut app, 6, 6));
    assert!(depot(&mut app, 10, 2));
    assert!(depot(&mut app, 2, 10));

    // A long ramp's upper half is at the plateau's level, and still a ramp:
    // (9, 6) to (10, 7) all stand at 2, two of them ramp cells.
    let mut app = hills_app(MovementModel::Cell, Ramps::Long, Rise::One);
    assert!(!depot(&mut app, 9, 6));
}

//
// ─── Reach ────────────────────────────────────────────────────────────────────
//

#[test]
fn cliff_walker_strikes_across_cliff() {
    // The post stands on the cliff's high edge, the strider on its low one.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (post, post_id) = utils::create_owned(&mut app, "post", 8, 2, 1);
    let (strider, strider_id) = utils::create_owned(&mut app, "strider", 7, 2, 0);
    utils::attack(&mut app, strider_id, post_id);
    utils::run_ticks(&mut app, 3);

    // The first strike lands on the third tick, from where the strider
    // stands: 100 − 10 = 90.
    assert_eq!(utils::cell_of(app.world(), strider), CellPos::new(7, 2));
    assert_eq!(utils::cell_of(app.world(), post), CellPos::new(8, 2));
    assert_eq!(utils::health_as_u32(&app, post), 90);
}

#[test]
fn walker_ordered_across_cliff_corner_strikes_from_ramp_top() {
    for (model, struck_from) in [
        (MovementModel::Cell, utils::pos(8, 6)),
        (MovementModel::Continuous, utils::part_way("7.5", "6")),
    ] {
        // The walker on the ramp's low cell (7, 6); the post on the cliff's
        // high cell (8, 5), a diagonal away across the cliff's corner. It
        // strikes only once it stands on the ramp's top, (8, 6), beside the
        // post at its level: on the cell under the cell model, and halfway
        // over it under the continuous one, its footprint reaching into it.
        let mut app = hills_app(model, Ramps::Short, Rise::One);
        let (post, post_id) = utils::create_owned(&mut app, "post", 8, 5, 1);
        let (walker, walker_id) = utils::create_owned(&mut app, "walker", 7, 6, 0);
        utils::attack(&mut app, walker_id, post_id);

        // The order is in on tick 3, when the chase starts its step east at
        // half a cell a tick; the walker is at 7.5 on tick 4 and switches to
        // the attack on tick 5 — on (8, 6) under the cell model, still at 7.5
        // under the continuous one — whose damage point, one tick in, lands on
        // tick 6 under either: 100 − 10 = 90.
        utils::run_ticks(&mut app, 5);
        assert_eq!(utils::health_as_u32(&app, post), 100, "{model:?}");
        utils::run_ticks(&mut app, 1);
        assert_eq!(utils::health_as_u32(&app, post), 90, "{model:?}");
        assert_eq!(
            utils::position_of(app.world(), walker),
            struck_from,
            "{model:?}"
        );
    }
}

//
// ─── Engaging and leaving ─────────────────────────────────────────────────────
//

#[test]
fn stand_ground_melee_never_acquires_across_cliff_corner() {
    // The walker on the ramp's low cell (7, 6); the post on the cliff's high
    // cell (8, 5), a diagonal away, its one path by (7, 5) up the cliff.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (post, _) = utils::create_owned(&mut app, "post", 8, 5, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "walker", 7, 6, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    // It never takes the post up as a target, and so never walks to it.
    for _ in 0..20 {
        utils::run_ticks(&mut app, 1);
        let orders = app.world().get::<OrderQueueComponent>(walker).unwrap();
        assert!(orders.front().is_none(), "it acquired the post");
    }
    assert_eq!(utils::cell_of(app.world(), walker), CellPos::new(7, 6));
    assert_eq!(utils::health_as_u32(&app, post), 100);
}

#[test]
fn static_body_left_on_ramp_stands_on_flat_ground() {
    // A walker dying on the ramp's low cell (7, 6) leaves a cairn, a static
    // body that stands only on flat ground: the ramp is refused and the
    // search's next cell, (6, 6) beside it, takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (walker, _) = utils::create_owned(&mut app, "cairn_bearer", 7, 6, 0);
    spawn::despawn_entity(app.world_mut(), walker, DeathCause::Decayed);
    utils::run_ticks(&mut app, 2);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(6, 6)]);
}

#[test]
fn trained_static_body_is_set_down_on_flat_ground() {
    // A hut at (6, 8) below the long ramp trains a cairn, a static body. The
    // first cell its search tries, (6, 7) above it, is a ramp cell and refused;
    // the cardinals on row 8 come next, (5, 8) first.
    let mut app = hills_app(MovementModel::Cell, Ramps::Long, Rise::One);
    let (_, hut_id) = utils::create_owned(&mut app, "hut", 6, 8, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: hut_id,
            type_name: "cairn".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(5, 8)]);
}

//
// ─── Changing form ────────────────────────────────────────────────────────────
//

#[test]
fn walker_on_ramp_never_changes_through_static_form() {
    // The chrysalis is as solid as the grower but static, so it is judged on
    // the ground: the ramp's (7, 6) refuses it, the flat (6, 6) takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (on_ramp, _) = utils::create_owned(&mut app, "grower", 7, 6, 0);
    let (on_flat, _) = utils::create_owned(&mut app, "grower", 6, 6, 0);
    let order = Order::Morph {
        type_name: "cairn".into(),
    };

    assert_eq!(
        orders::can_start(app.world(), on_ramp, &order),
        Err(Refusal::NoRoom)
    );
    assert_eq!(orders::can_start(app.world(), on_flat, &order), Ok(()));
}

#[test]
fn reserving_change_through_static_interim_claims_where_it_lands() {
    // A seedling at x 3.7 settles its static seed pod on the cell its
    // position rounds to, (4, 2), and the 2×2 grove the change reserves
    // settles from the pod, as the landing does: (4, 2) − ½ = (3.5, 1.5),
    // rounding to (4, 2), over x 4 and 5. A cairn at (5, 2) refuses it; from
    // 3.7 alone the grove would settle at (3.2, 1.5), rounding to (3, 2),
    // over x 3 and 4, clear of the cairn.
    let mut app = hills_app(MovementModel::Continuous, Ramps::Short, Rise::One);
    let (seedling, _) = utils::create_owned(&mut app, "seedling", 3, 2, 0);
    app.world_mut()
        .get_mut::<LocationComponent>(seedling)
        .unwrap()
        .position = utils::part_way("3.7", "2");
    utils::run_ticks(&mut app, 1);
    let order = Order::Morph {
        type_name: "grove".into(),
    };
    assert_eq!(orders::can_start(app.world(), seedling, &order), Ok(()));

    utils::create_owned(&mut app, "cairn", 5, 2, 0);
    assert_eq!(
        orders::can_start(app.world(), seedling, &order),
        Err(Refusal::NoRoom)
    );
}

#[test]
fn walker_on_ramp_never_changes_into_static_form() {
    // A settler reserves the ground its cairn will stand on: a static body,
    // which stands only on flat ground, so the ramp's (7, 6) refuses it and
    // the flat (6, 6) beside it takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (on_ramp, _) = utils::create_owned(&mut app, "settler", 7, 6, 0);
    let (on_flat, _) = utils::create_owned(&mut app, "settler", 6, 6, 0);
    let order = Order::Morph {
        type_name: "cairn".into(),
    };

    assert_eq!(
        orders::can_start(app.world(), on_ramp, &order),
        Err(Refusal::NoRoom)
    );
    assert_eq!(orders::can_start(app.world(), on_flat, &order), Ok(()));
}

#[test]
fn larva_changing_from_berth_on_cliff_steps_out_where_its_own_form_stands() {
    // The nest's one berth sits on the cliff's low edge at (4, 7), off a
    // walking larva's ground though the air form it changes into flies over
    // it; changing straight, the larva stands on the step-out cell itself.
    // Around the berth the nest holds (4, 6), the band holds rows 7 and 8, so
    // the first cell the larva takes is the first ring's diagonal (3, 6) on
    // low ground.
    let mut app = nest_app();
    utils::create_owned(&mut app, "nest", 4, 4, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 7));

    utils::order_morph(&mut app, larva, "drone");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), larva));
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(3, 6));
}

#[test]
fn underfoot_pod_steps_out_onto_its_own_berth() {
    // The pod holds ground underfoot and cannot move, so its change asks what
    // stands underfoot; at its own berth, (4, 4) below the nest on low ground,
    // it is the only thing there and must not count against itself.
    let mut app = nest_app();
    utils::create_owned(&mut app, "pod_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let pod = brood_of(&app, "pod");
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(4, 4));

    utils::order_morph(&mut app, pod, "bulb");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), pod));
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(4, 4));
}

#[test]
fn larva_changing_straight_steps_out_under_flier_holding_air() {
    // An idle drone holds the air over (3, 6); the larva changing straight
    // into a drone stands there as a larva through the change, which asks
    // nothing of the air: it still steps out at (3, 6).
    let mut app = nest_app();
    utils::create_owned(&mut app, "nest", 4, 4, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");
    utils::create_owned(&mut app, "drone", 3, 6, 0);

    utils::order_morph(&mut app, larva, "drone");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), larva));
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(3, 6));
}

#[test]
fn underfoot_pod_never_steps_out_onto_another_underfoot_body() {
    // A rug lies underfoot at the pod's berth (4, 4): the berth is refused,
    // the nest holds (4, 3) above it, and (3, 4) on row 4 comes next.
    let mut app = nest_app();
    utils::create_owned(&mut app, "rug", 4, 4, 0);
    utils::create_owned(&mut app, "pod_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let pod = brood_of(&app, "pod");
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(4, 4));

    utils::order_morph(&mut app, pod, "bulb");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), pod));
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(3, 4));
}

#[test]
fn underfoot_pod_reserving_static_form_steps_out_onto_its_own_berth() {
    // The reserving change judges the sealed bulb where the pod stepped out,
    // its own berth (4, 4), and the pod is not in its own way there either.
    let mut app = nest_app();
    utils::create_owned(&mut app, "pod_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let pod = brood_of(&app, "pod");

    utils::order_morph(&mut app, pod, "sealed_bulb");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), pod));
    assert!(app.world().get::<MorphComponent>(pod).is_some());
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(4, 4));
}

#[test]
fn pod_reserving_form_held_to_field_steps_out_where_field_covers() {
    // The mulcher at (6, 5) covers the cells within one of it — its four
    // neighbors (6, 4), (5, 5), (7, 5) and (6, 6) — with mulch, which the
    // sprout must stand on. Around the berth (4, 4) the nest holds (4, 3) and
    // the cardinals (3, 4), (5, 4) and (4, 5) lie outside the mulch; of the
    // diagonals by row, (3, 3) and (3, 5) lie outside it, (5, 3) is the
    // nest's, and (5, 5) inside it takes the pod.
    let mut app = nest_app();
    utils::create_owned(&mut app, "mulcher", 6, 5, 0);
    utils::create_owned(&mut app, "pod_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 2);
    let pod = brood_of(&app, "pod");

    utils::order_morph(&mut app, pod, "sprout");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::attached(app.world(), pod));
    assert_eq!(utils::cell_of(app.world(), pod), CellPos::new(5, 5));
}

#[test]
fn larva_changing_through_husk_steps_out_where_husk_stands() {
    // Through a husk, a mover that strides cliffs, the change stands the husk
    // on the step-out cell and never the larva: the berth's own cliff cell
    // (4, 7) takes it.
    let mut app = nest_app();
    utils::create_owned(&mut app, "nest", 4, 4, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");

    utils::order_morph(&mut app, larva, "glider");
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::of(app.world(), larva).name, "husk");
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 7));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Which ramp a [`hills_app`] map has on rows 6 and 7, rising east.
#[derive(Debug, Clone, Copy)]
enum Ramps {
    /// None: the cliff runs the plateau's whole edge and cuts it off.
    None,
    /// Two long: column 7 on low ground, column 8 on the plateau's edge.
    Short,
    /// Four long: columns 6 and 7 on low ground, 8 and 9 on the plateau.
    Long,
}

/// How far [`hills_app`]'s plateau rises above low ground.
#[derive(Debug, Clone, Copy)]
enum Rise {
    /// One level: the plateau at 2.
    One,
    /// Two levels: the plateau at 3, its cliff too tall for any walker.
    Two,
}

/// Two hostile humans on a 16×16 land map under `model`: low ground at 1 to
/// column 7, a plateau `rise` above it from column 8, a cliff band rising
/// east across columns 7 and 8 but where `ramps` lies; the ground spans
/// levels 1 to 3.
///
/// - `walker` and `strider` are 1×1 movers whose melee hits for 10 every 2
///   ticks, the walker stopped by cliffs, the strider walking them;
///   `big_walker` and `big_strider` are 2×2 movers of the same domains;
/// - `post` is an unarmed 1×1 target of 100 health that strides cliffs,
///   `depot` a 2×2 building;
/// - `cairn_bearer` is a walker that leaves a `cairn`, a static 1×1 body a
///   static 1×1 `hut` also trains and a `settler` walker changes into,
///   reserving its ground;
/// - `grower` is a walker changing through a static `chrysalis` into a
///   `cairn`, and `seedling` one changing through a static `seed_pod` into a
///   static 2×2 `grove`, reserving its ground;
/// - `beacon` is a static 1×1 body on the air layer, flat at 4.
fn hills_app(model: MovementModel, ramps: Ramps, rise: Rise) -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground", LayerLevel::Range { from: 1, to: 3 });
    let air = registry.register_layer("air", LayerLevel::At(4));
    registry.register_terrain("land");
    let aloft = registry.register_domain("aloft", DomainDef::new(air, Terrains::Any, None));
    let walking = registry.register_domain(
        "walking",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Ramp)),
    );
    let striding = registry.register_domain(
        "striding",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Cross)),
    );
    for (name, domain, size) in [
        ("walker", walking, 1),
        ("strider", striding, 1),
        ("big_walker", walking, 2),
        ("big_strider", striding, 2),
    ] {
        registry.register(mover(name, domain, ground, size));
    }
    registry.register(
        EntityTypeDef::new("post")
            .with_location(striding, CellSize::ONE, Solidity::Solid, ground)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(100))
            .with_dying(1, []),
    );
    registry
        .register(mover("cairn_bearer", walking, ground, 1).with_dying(1, utils::leaves("cairn")));
    registry.register(
        EntityTypeDef::new("cairn")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_train_time(2),
    );
    registry.register(
        mover("settler", walking, ground, 1)
            .with_morphs([change("cairn", MorphPlacement::Reserve)]),
    );
    registry.register(
        mover("grower", walking, ground, 1).with_morphs([change_through(
            "chrysalis",
            "cairn",
            MorphPlacement::Revalidate,
        )]),
    );
    registry.register(
        mover("seedling", walking, ground, 1).with_morphs([change_through(
            "seed_pod",
            "grove",
            MorphPlacement::Reserve,
        )]),
    );
    for (name, size) in [("chrysalis", 1), ("seed_pod", 1), ("grove", 2)] {
        registry.register(
            EntityTypeDef::new(name)
                .with_location(walking, CellSize::new(size, size), Solidity::Solid, ground)
                .with_pool(Pool::health(50))
                .with_dying(1, []),
        );
    }
    registry.register(
        EntityTypeDef::new("hut")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_trainer(["cairn"]),
    );
    registry.register(
        EntityTypeDef::new("beacon")
            .with_location(aloft, CellSize::ONE, Solidity::Solid, air)
            .with_pool(Pool::health(50)),
    );
    registry.register(
        EntityTypeDef::new("depot")
            .with_location(walking, CellSize::new(2, 2), Solidity::Solid, ground)
            .with_pool(Pool::health(100))
            .with_tags(["building"]),
    );
    registry.validate();

    let mut data = MapData::new("hills", Projection::Isometric, 16, 16, "land");
    data.set_movement_model(model);
    data.add_relief("ground", 1);
    let plateau = match rise {
        Rise::One => 2,
        Rise::Two => 3,
    };
    for y in 0..16 {
        for x in 8..16 {
            data.set_cell_level("ground", (x, y), plateau);
        }
    }
    for y in 0..16 {
        for x in [7, 8] {
            data.set_slope("ground", (x, y), Slope::Cliff(Direction::East));
        }
    }
    let ramp_columns = match ramps {
        Ramps::None => 0..0,
        Ramps::Short => 7..9,
        Ramps::Long => 6..10,
    };
    for y in [6, 7] {
        for x in ramp_columns.clone() {
            data.set_slope("ground", (x, y), Slope::Ramp(Direction::East));
        }
    }
    let live = Map::from_data(&data, &registry);
    app.insert_resource(registry);
    map::install_map(app.world_mut(), live);
    utils::start_game(&mut app);
    app
}

/// A `size`-square mover of `domain` on `ground`, its melee hitting for 10
/// every 2 ticks.
fn mover(name: &str, domain: DomainId, ground: LayerId, size: u32) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(domain, CellSize::new(size, size), Solidity::Solid, ground)
        .with_movement(
            utils::fixed("0.5"),
            utils::fixed("0.5") * FixedU64::from_num(size),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_pool(Pool::health(30))
        .with_dying(1, [])
        .with_attack(utils::weapon(ground), 10, 1, 3, 2, 1)
        .with_sight_range(5)
}

/// One human on a 12×12 land map: low ground at 1 to row 7, a plateau at 2
/// from row 8, a cliff band rising south across rows 7 and 8; the ground
/// spans levels 1 to 2 and the air is flat at 3.
///
/// - `nest` is a 3×3 building with one berth in the middle of the cell below
///   its west column, opening with one `larva` there; `pod_nest` the same,
///   opening with one `pod`;
/// - `larva` is a passable walker that keeps its berth and changes straight
///   into a `drone`, a flier, or through a `husk`, a mover that strides
///   cliffs, into a `glider`, another flier;
/// - `pod` is an underfoot walker with no movement that changes straight into
///   a solid `bulb`, into a `sealed_bulb` reserving its ground, or into a
///   `sprout` reserving ground it may hold only on `mulch`;
/// - `mulcher` spreads mulch one cell around itself, and `rug` is a static
///   underfoot body.
fn nest_app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground", LayerLevel::Range { from: 1, to: 2 });
    let air = registry.register_layer("air", LayerLevel::At(3));
    registry.register_terrain("land");
    let aloft = registry.register_domain("aloft", DomainDef::new(air, Terrains::Any, None));
    let walking = registry.register_domain(
        "walking",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Ramp)),
    );
    let striding = registry.register_domain(
        "striding",
        DomainDef::new(ground, Terrains::Any, Some(CliffCrossing::Cross)),
    );
    let mulch = registry.register_field(
        "mulch",
        FieldDef::new(
            Terrains::Any,
            ground.into(),
            FieldDecay::Instant,
            FieldVision::Dark,
            Detection::Blind,
        ),
    );
    registry.register(
        EntityTypeDef::new("nest")
            .with_location(walking, CellSize::new(3, 3), Solidity::Solid, ground)
            .with_pool(Pool::health(300))
            .with_dying(2, [])
            .with_tags(["building"])
            .with_berths([("brood", BerthGroup::new([utils::berth("0.5", "3.5")], 1))])
            .with_breeder("larva", Quantity::Constant(10), 1, 1, OrphanFate::Perish),
    );
    registry.register(
        EntityTypeDef::new("larva")
            .with_location(walking, CellSize::ONE, Solidity::Passable, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, [])
            .with_broodling(Attachment::new("brood", BerthStance::Still))
            .with_morphs([
                change("drone", MorphPlacement::Nearby),
                change_through("husk", "glider", MorphPlacement::Nearby),
            ]),
    );
    registry.register(
        EntityTypeDef::new("pod_nest")
            .with_location(walking, CellSize::new(3, 3), Solidity::Solid, ground)
            .with_pool(Pool::health(300))
            .with_dying(2, [])
            .with_tags(["building"])
            .with_berths([("brood", BerthGroup::new([utils::berth("0.5", "3.5")], 1))])
            .with_breeder("pod", Quantity::Constant(10), 1, 1, OrphanFate::Perish),
    );
    registry.register(
        EntityTypeDef::new("pod")
            .with_location(walking, CellSize::ONE, Solidity::Underfoot, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, [])
            .with_broodling(Attachment::new("brood", BerthStance::Still))
            .with_morphs([
                change("bulb", MorphPlacement::Revalidate),
                change("sealed_bulb", MorphPlacement::Reserve),
                change("sprout", MorphPlacement::Reserve),
            ]),
    );
    for name in ["bulb", "sealed_bulb"] {
        registry.register(
            EntityTypeDef::new(name)
                .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
                .with_pool(Pool::health(25))
                .with_dying(1, []),
        );
    }
    registry.register(
        EntityTypeDef::new("sprout")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, [])
            .with_field_placement([FieldPlacement::Requires {
                field: mulch,
                of: Affiliation::Anyone,
                coverage: FieldCoverage::Every,
            }]),
    );
    registry.register(
        EntityTypeDef::new("mulcher")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, [])
            .with_field_sources([FieldSourceDef::new(
                mulch,
                1,
                FieldGrowth::Instant,
                Emission::Nothing,
                Emission::Full,
            )]),
    );
    registry.register(
        EntityTypeDef::new("rug")
            .with_location(walking, CellSize::ONE, Solidity::Underfoot, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, []),
    );
    registry.register(
        EntityTypeDef::new("husk")
            .with_location(striding, CellSize::ONE, Solidity::Solid, ground)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(25))
            .with_dying(1, []),
    );
    registry.register(
        EntityTypeDef::new("glider")
            .with_location(aloft, CellSize::ONE, Solidity::Solid, air)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(25))
            .with_dying(1, []),
    );
    registry.register(
        EntityTypeDef::new("drone")
            .with_location(aloft, CellSize::ONE, Solidity::Solid, air)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(25))
            .with_dying(1, []),
    );
    registry.validate();

    let mut data = MapData::new("nest", Projection::Isometric, 12, 12, "land");
    data.add_relief("ground", 1);
    for x in 0..12 {
        for y in 8..12 {
            data.set_cell_level("ground", (x, y), 2);
        }
        for y in [7, 8] {
            data.set_slope("ground", (x, y), Slope::Cliff(Direction::South));
        }
    }
    let live = Map::from_data(&data, &registry);
    app.insert_resource(registry);
    map::install_map(app.world_mut(), live);
    utils::start_game(&mut app);
    app
}

/// The one broodling of type `name` a [`nest_app`] breeder opens with.
fn brood_of(app: &App, name: &str) -> Entity {
    app.world()
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .find(|&entity| entity_def::of(app.world(), entity).name == name)
        .expect("the breeder opens with its broodling")
}

/// The cells every alive body of type `name` stands on, in index order.
fn cells_of(app: &App, name: &str) -> Vec<CellPos> {
    app.world()
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .filter(|&entity| entity_def::of(app.world(), entity).name == name)
        .map(|entity| utils::cell_of(app.world(), entity))
        .collect()
}

/// A free, refundable change straight into `into` after 10 ticks, placed as
/// `placement` says.
fn change(into: &str, placement: MorphPlacement) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::direct(MorphInterrupted::Reverts),
        Quantity::Constant(10),
        placement,
        MorphCancel::Refundable,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}

/// A free, refundable change into `into` through the interim form `via`
/// after 10 ticks, placed as `placement` says.
fn change_through(via: &str, into: &str, placement: MorphPlacement) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::via(
            via,
            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ViaInterrupted::reverts([(
                PoolId::HEALTH,
                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
            )]),
        ),
        Quantity::Constant(10),
        placement,
        MorphCancel::Refundable,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}
