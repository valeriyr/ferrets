//! Levels: a plateau climbed by its ramp, cliffs that stop all but cliff walkers, footprints across level edges, reach across them, and bodies set down and changing form on them.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::map;
use ferrets_content::{
    affiliation::Affiliation,
    attack::Slain,
    berths::BerthGroup,
    brood::OrphanFate,
    detection::Detection,
    domain::{DomainDef, DomainId},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldCoverage, FieldDecay, FieldDef, FieldGrowth, FieldPlacement, FieldSourceDef,
        FieldVision,
    },
    gateway::{Doors, Joins},
    kinds::Kinds,
    landing::Landing,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    network::NetworkDef,
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{
        Casting, EntityCastEffect, Reach, SkillCaster, SkillDef, SkillId, SummonArrival, SummonAt,
        SummonPlacement,
    },
    terrains::Terrains,
    transport::{PassengerConduct, PassengerFate},
    work::{Attachment, BerthStance},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing,
    layer_id::LayerId,
    layer_level::LayerLevel,
    slope::{Direction, Slope},
};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{location::LocationComponent, order_queue::OrderQueueComponent, stance::Stance},
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent},
    game_loop::{
        damage,
        orders::{self, Refusal},
    },
    make_way,
    map::Map,
    map_data::MapData,
    movement_model::MovementModel,
    order::Order,
    session::{player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn::{self, Boarding, FieldReach},
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

#[test]
fn unit_trained_at_cliff_edge_never_lands_below_it() {
    // A hut at (9, 0) on the plateau's edge, the cliff band west of it and no
    // ramp: walkers hold every plateau cell within three rings of it, (9, 1)
    // to (9, 3) and four 2×2 bodies over (10, 0) to (13, 3). The lowland at
    // (6, 0), three rings out, is free but out of the region the recruit
    // steps into from the hut; the fourth ring's plateau cells come next,
    // (9, 4) nearest the hut's middle.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, hut_id) = utils::create_owned(&mut app, "hut", 9, 0, 0);
    for y in 1..4 {
        utils::create_owned(&mut app, "walker", 9, y, 0);
    }
    for (x, y) in [(10, 0), (12, 0), (10, 2), (12, 2)] {
        utils::create_owned(&mut app, "big_walker", x, y, 0);
    }
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: hut_id,
            type_name: "recruit".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "recruit"), vec![CellPos::new(9, 4)]);
}

#[test]
fn unit_set_down_from_above_takes_highest_level_first() {
    // An aerie hovers at (7, 0) over the cliff band, its west column over the
    // lowland's level 1 and its east one over the plateau's level 2. Its
    // recruit comes down from the air: the first ring holds (6, 0) below the
    // cliff and (9, 0) above it, as near the aerie's middle; the highest
    // level under the aerie, 2, comes first.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, aerie_id) = utils::create_owned(&mut app, "aerie", 7, 0, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: aerie_id,
            type_name: "recruit".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "recruit"), vec![CellPos::new(9, 0)]);
}

#[test]
fn passenger_let_out_from_above_takes_highest_level_first() {
    // An airship hovers at (7, 0) over the cliff band, on the air layer above
    // the recruit's ground. Let out within one cell, the recruit comes down
    // from above: (6, 0) below the cliff and (9, 0) above it are as near, and
    // the plateau's level 2 comes first.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (airship, airship_id) = utils::create_owned(&mut app, "airship", 7, 0, 0);
    let (recruit, _) = utils::create_owned(&mut app, "recruit", 3, 3, 0);
    assert_eq!(
        spawn::board(app.world_mut(), recruit, airship),
        Boarding::Boarded
    );

    utils::unload(&mut app, airship_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), recruit), CellPos::new(9, 0));
}

#[test]
fn passenger_let_out_from_above_takes_lower_level_when_highest_is_full() {
    // The airship as above, with walkers on the plateau's cells within its one
    // cell of reach, (9, 0) to (9, 2): the highest level has no room, and the
    // other levels come next — (6, 0) below the cliff, first by row.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (airship, airship_id) = utils::create_owned(&mut app, "airship", 7, 0, 0);
    for y in 0..3 {
        utils::create_owned(&mut app, "walker", 9, y, 0);
    }
    let (recruit, _) = utils::create_owned(&mut app, "recruit", 3, 3, 0);
    assert_eq!(
        spawn::board(app.world_mut(), recruit, airship),
        Boarding::Boarded
    );

    utils::unload(&mut app, airship_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::cell_of(app.world(), recruit), CellPos::new(6, 0));
}

#[test]
fn unit_trained_underground_gets_no_high_ground_preference() {
    // A den on the deep layer under the cliff band, (7, 0): its recruit comes
    // up from below, so no level goes first — (6, 0) below the cliff and
    // (9, 0) above it are as near, and the row and column order takes (6, 0).
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, den_id) = utils::create_owned(&mut app, "den", 7, 0, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: den_id,
            type_name: "recruit".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "recruit"), vec![CellPos::new(6, 0)]);
}

#[test]
fn static_body_trained_at_cliff_edge_never_lands_below_it() {
    // The hut at (9, 0) on the plateau's edge, its plateau cells within three
    // rings held as for the recruit above. A cairn never walks, so no region
    // binds it, but it lands only at a level it steps onto from the hut: the
    // lowland's (6, 0) three rings out is refused, and the fourth ring's
    // plateau cell nearest the hut's middle, (9, 4), takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, hut_id) = utils::create_owned(&mut app, "hut", 9, 0, 0);
    for y in 1..4 {
        utils::create_owned(&mut app, "walker", 9, y, 0);
    }
    for (x, y) in [(10, 0), (12, 0), (10, 2), (12, 2)] {
        utils::create_owned(&mut app, "big_walker", x, y, 0);
    }
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: hut_id,
            type_name: "cairn".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(9, 4)]);
}

#[test]
fn unit_summoned_at_point_lands_in_its_region() {
    // A walker is called to (6, 0) on the lowland, whose free cells nearest
    // it are taken by four 2×2 bodies over (3, 0) to (6, 3). The third ring
    // reaches the plateau at (9, 0), up the cliff with no ramp: out of the
    // point's region. The fourth ring's lowland cells come next, (2, 0) and
    // (6, 4) as near the point, the row first.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    for (x, y) in [(3, 0), (5, 0), (3, 2), (5, 2)] {
        utils::create_owned(&mut app, "big_walker", x, y, 0);
    }
    let (_, invoker) = utils::create_owned(&mut app, "invoker", 1, 12, 0);
    utils::cast_at(&mut app, invoker, "call_walker", 6, 0);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::owned_of_type(app.world_mut(), "walker", 0)
            .into_iter()
            .map(|walker| utils::cell_of(app.world(), walker))
            .collect::<Vec<_>>(),
        vec![CellPos::new(2, 0)]
    );
}

#[test]
fn holder_letting_out_at_range_never_unloads_up_cliff() {
    // A wagon at (6, 0) at the cliff band's foot lets its recruit out within
    // three cells. Walkers hold every lowland cell in range, (3, 0) to (6, 3)
    // but the wagon's own; the plateau's (9, 0) to (9, 3) lie three cells off,
    // in range, but up a cliff the recruit never steps from the wagon's ring:
    // it stays aboard.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 6, 0, 0);
    for y in 0..4 {
        for x in 3..7 {
            if (x, y) != (6, 0) {
                utils::create_owned(&mut app, "walker", x, y, 0);
            }
        }
    }
    let (recruit, _) = utils::create_owned(&mut app, "recruit", 3, 8, 0);
    assert_eq!(
        spawn::board(app.world_mut(), recruit, wagon),
        Boarding::Boarded
    );

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(entity_def::boarded(app.world(), recruit));
}

#[test]
fn gateway_letting_out_toward_point_up_cliff_lets_out_only_what_steps_from_it() {
    // A burrow at (6, 0) at the cliff band's foot holds a recruit and lets it
    // out toward (9, 0) on the plateau, three cells off, in range. Walkers
    // hold every lowland cell in range; the plateau's lie up a cliff the
    // recruit never steps from the burrow's ring: it stays inside.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, burrow_id) = utils::create_owned(&mut app, "burrow", 6, 0, 0);
    let (recruit, recruit_id) = utils::create_owned(&mut app, "recruit", 6, 2, 0);
    utils::send_to(&mut app, recruit_id, burrow_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert!(entity_def::hidden(app.world(), recruit));
    for y in 0..4 {
        for x in 3..7 {
            if (x, y) != (6, 0) {
                utils::create_owned(&mut app, "walker", x, y, 0);
            }
        }
    }

    utils::unload(&mut app, burrow_id, Some(utils::pos(9, 0)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(entity_def::hidden(app.world(), recruit));
}

#[test]
fn static_body_left_from_above_takes_highest_level_first() {
    // A sky shrine hovers at (7, 0) over the cliff band and leaves a cairn, a
    // static body coming down from the air: (6, 0) below the cliff and
    // (9, 0) above it are as near, and the highest level under the shrine,
    // the plateau's 2, comes first.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (shrine, _) = utils::create_owned(&mut app, "sky_shrine", 7, 0, 0);
    spawn::despawn_entity(app.world_mut(), shrine, DeathCause::Decayed);
    utils::run_ticks(&mut app, 2);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(9, 0)]);
}

#[test]
fn static_body_coming_up_from_below_lands_only_at_level_it_steps_onto() {
    // A den under the lowland at (5..6, 0..1) trains a cairn. Every ground
    // cell is held but (9, 0) on the plateau, three rings out, and (2, 5) on
    // the lowland, four out: coming up from below, the cairn lands only at a
    // level it steps onto across the den's edge, the lowland's, at (2, 5).
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, den_id) = utils::create_owned(&mut app, "den", 5, 0, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(9, 0));
    utils::free_cell(&mut app, CellPos::new(2, 5));
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: den_id,
            type_name: "cairn".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(2, 5)]);
}

#[test]
fn static_body_coming_down_from_above_lands_at_any_level() {
    // An aerie over the lowland at (4..5, 0..1) trains a cairn. Every ground
    // cell is held but (9, 0) on the plateau, four rings out: coming down
    // from the air, the cairn lands at any level, and takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    let (_, aerie_id) = utils::create_owned(&mut app, "aerie", 4, 0, 0);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(9, 0));
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: aerie_id,
            type_name: "cairn".into(),
        },
    );
    utils::run_ticks(&mut app, 10);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(9, 0)]);
}

#[test]
fn static_body_summoned_at_point_never_lands_below_cliff() {
    // A cairn is called to (9, 0) on the plateau's edge, held by a hut, its
    // plateau cells within three rings held as for the training above. A
    // cairn never walks, but it lands only at a level it steps onto from the
    // point's cell: the lowland's (6, 0) three rings out is refused, and the
    // fourth ring's plateau cell nearest the point, (9, 4), takes it.
    let mut app = hills_app(MovementModel::Cell, Ramps::None, Rise::One);
    utils::create_owned(&mut app, "hut", 9, 0, 0);
    for y in 1..4 {
        utils::create_owned(&mut app, "walker", 9, y, 0);
    }
    for (x, y) in [(10, 0), (12, 0), (10, 2), (12, 2)] {
        utils::create_owned(&mut app, "big_walker", x, y, 0);
    }
    let (_, invoker) = utils::create_owned(&mut app, "invoker", 1, 12, 0);
    utils::cast_at(&mut app, invoker, "call_cairn", 9, 0);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(cells_of(&app, "cairn"), vec![CellPos::new(9, 4)]);
}

//
// ─── Changing form ────────────────────────────────────────────────────────────
//

#[test]
fn reserving_change_moved_off_its_ground_lands_only_where_ground_takes_it() {
    // A settler at (3, 3) reserves its cell for a cairn, a static body that
    // stands only on flat ground. Moved during the change onto the ramp's
    // (7, 6), it settles there when the change runs out: the ramp refuses the
    // cairn, and the change ends back in the settler's form. Its twin left
    // where it reserved lands as a cairn.
    let mut app = hills_app(MovementModel::Continuous, Ramps::Short, Rise::One);
    let (moved, _) = utils::create_owned(&mut app, "settler", 3, 3, 0);
    let (stayed, _) = utils::create_owned(&mut app, "settler", 3, 10, 0);
    utils::order_morph(&mut app, moved, "cairn");
    utils::order_morph(&mut app, stayed, "cairn");
    utils::run_ticks(&mut app, 3);

    app.world_mut()
        .get_mut::<LocationComponent>(moved)
        .expect("a settler stands on the map")
        .position = utils::pos(7, 6);
    utils::run_ticks(&mut app, 12);

    assert_eq!(entity_def::type_name(app.world(), moved), "settler");
    assert_eq!(entity_def::type_name(app.world(), stayed), "cairn");
}

#[test]
fn reserving_change_into_walkable_form_lands_under_standing_units() {
    // A weaver at (3, 3) changes into a mat lying underfoot over a 2×2, with
    // walkers of its own side at (2, 3) and (4, 3), one of which stands on the
    // mat's ground however it recenters. The mat claims no cells, so it holds
    // none ahead, shares its ground with them, and lands.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (weaver, _) = utils::create_owned(&mut app, "weaver", 3, 3, 0);
    let (west, _) = utils::create_owned(&mut app, "walker", 2, 3, 0);
    let (east, _) = utils::create_owned(&mut app, "walker", 4, 3, 0);

    utils::order_morph(&mut app, weaver, "mat");
    utils::run_ticks(&mut app, 12);

    assert_eq!(entity_def::type_name(app.world(), weaver), "mat");
    assert_eq!(utils::cell_of(app.world(), weaver), CellPos::new(3, 3));
    assert_eq!(utils::cell_of(app.world(), west), CellPos::new(2, 3));
    assert_eq!(utils::cell_of(app.world(), east), CellPos::new(4, 3));
}

#[test]
fn reserving_change_into_passable_form_lands_under_standing_units() {
    // The weaver's change as above, a netter changing into a net, a static
    // 2×2 passable body: it claims no cells either, holds none ahead, and
    // lands under the walkers.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (netter, _) = utils::create_owned(&mut app, "netter", 3, 3, 0);
    let (west, _) = utils::create_owned(&mut app, "walker", 2, 3, 0);
    let (east, _) = utils::create_owned(&mut app, "walker", 4, 3, 0);

    utils::order_morph(&mut app, netter, "net");
    utils::run_ticks(&mut app, 12);

    assert_eq!(entity_def::type_name(app.world(), netter), "net");
    assert_eq!(utils::cell_of(app.world(), netter), CellPos::new(3, 3));
    assert_eq!(utils::cell_of(app.world(), west), CellPos::new(2, 3));
    assert_eq!(utils::cell_of(app.world(), east), CellPos::new(4, 3));
}

#[test]
fn passable_static_body_rises_under_mover_claiming_nothing() {
    // A net is passable and claims no cells: a wisp walking claiming nothing
    // at (2, 2) keeps only solid footprints off, so the net's 2×2 is laid
    // under it, as under a walker that claims its cell.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    utils::create_owned(&mut app, "wisp", 2, 2, 0);

    let net = utils::create_entity(app.world_mut(), "net", utils::pos(2, 2), Some(0));

    assert!(net.is_some());
}

#[test]
fn solid_static_footprint_never_rises_over_mover_claiming_nothing() {
    // A wisp walks claiming no cells; standing at (2, 2), it keeps a cairn, a
    // solid static body, from being placed over it, so it never stands where its
    // shape has no region. Beside it, (2, 3) takes the cairn.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    utils::create_owned(&mut app, "wisp", 2, 2, 0);

    let over = spawn::spawn_placed(
        app.world_mut(),
        "cairn",
        utils::pos(2, 2),
        Some(0),
        FieldReach::Full,
        &[],
    );
    let beside = spawn::spawn_placed(
        app.world_mut(),
        "cairn",
        utils::pos(2, 3),
        Some(0),
        FieldReach::Full,
        &[],
    );

    assert!(over.is_none());
    assert!(beside.is_some());
}

#[test]
fn solid_static_footprint_never_rises_over_any_cell_mover_claiming_nothing_stands_across() {
    // A wisp half-way from (2, 2) to (3, 2) rounds to (3, 2) but stands
    // across both: a cairn is refused on either, and beside them, on (1, 2),
    // it stands.
    let mut app = hills_app(MovementModel::Continuous, Ramps::Short, Rise::One);
    let (wisp, _) = utils::create_owned(&mut app, "wisp", 2, 2, 0);
    app.world_mut()
        .get_mut::<LocationComponent>(wisp)
        .expect("the wisp stands")
        .position = FixedUVec2::new(utils::fixed("2.5"), utils::fixed("2"));
    let place = |app: &mut App, x| {
        utils::create_entity(app.world_mut(), "cairn", utils::pos(x, 2), Some(0)).is_some()
    };

    assert!(!place(&mut app, 2));
    assert!(!place(&mut app, 3));
    assert!(place(&mut app, 1));
}

#[test]
fn change_cut_short_puts_form_back_where_ground_takes_it() {
    // A kiln changing through a scaffold, cut short, is a kiln again on
    // (3, 3).
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (kiln, _) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), kiln), "scaffold");

    utils::soft_cancel_orders(app.world_mut(), kiln);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::type_name(app.world(), kiln), "kiln");
    assert_eq!(utils::cell_of(app.world(), kiln), CellPos::new(3, 3));
}

#[test]
fn change_cut_short_where_ground_no_longer_takes_its_form_destroys_it() {
    // The scaffold lies underfoot, so an enemy walker is placed on (3, 3)
    // while the kiln changes; cut short, the kiln, a solid static body, cannot
    // be put back over the walker, which does not make way for it, and is
    // destroyed.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (kiln, kiln_id) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), kiln), "scaffold");
    let (walker, _) = utils::create_owned(&mut app, "walker", 3, 3, 1);

    utils::soft_cancel_orders(app.world_mut(), kiln);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, kiln_id));
    assert_eq!(utils::cell_of(app.world(), walker), CellPos::new(3, 3));
}

#[test]
fn change_cut_short_sets_idle_ally_on_its_scaffold_down_beside_it() {
    // The same meeting with a walker of the kiln's own side, idle on the
    // scaffold at (3, 3): cut short, the change does not wait on it — it is
    // set down at once on the nearest cell clear of the kiln, (3, 2) above
    // it, and the kiln goes back where it stood.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (kiln, _) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), kiln), "scaffold");
    let (walker, _) = utils::create_owned(&mut app, "walker", 3, 3, 0);

    utils::soft_cancel_orders(app.world_mut(), kiln);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::type_name(app.world(), kiln), "kiln");
    assert_eq!(utils::cell_of(app.world(), kiln), CellPos::new(3, 3));
    assert_eq!(utils::cell_of(app.world(), walker), CellPos::new(3, 2));
}

#[test]
fn change_cut_short_sets_ally_walking_across_its_scaffold_down_beside_it() {
    // An own walker sent from (3, 1) to (3, 7) is crossing into the scaffold's
    // (3, 3), holding it, on the fifth tick as the change is cut short: it is
    // set down beside the kiln like an idle ally, the kiln goes back where it
    // stood, and the walk carries on to its goal.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (kiln, kiln_id) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), kiln), "scaffold");
    let (walker, walker_id) = utils::create_owned(&mut app, "walker", 3, 1, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(3, 7),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::part_way("3", "2.5")
    );

    utils::soft_cancel_orders(app.world_mut(), kiln);
    utils::run_ticks(&mut app, 1);

    assert!(utils::alive(&app, kiln_id));
    assert_eq!(entity_def::type_name(app.world(), kiln), "kiln");
    assert_eq!(utils::cell_of(app.world(), kiln), CellPos::new(3, 3));
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::cell_of(app.world(), walker), CellPos::new(3, 7));
    assert!(utils::order_queue_is_empty(app.world_mut(), walker));
}

#[test]
fn walker_killed_across_scaffold_no_longer_clears_it() {
    // The walker crossing into the scaffold's (3, 3) on the fifth tick is
    // killed: still walking until its death flushes the walk, it no longer
    // clears the ground it stands on.
    let (mut app, walker, kiln_id) = walker_across_scaffold();
    assert!(make_way::clears(app.world(), walker, Some(0)));

    damage::apply(
        app.world_mut(),
        kiln_id,
        walker,
        FixedU64::from_num(1000),
        Slain::Nothing,
    );

    assert!(entity_def::walking(app.world(), walker));
    assert!(!make_way::clears(app.world(), walker, Some(0)));
}

#[test]
fn walker_disabled_across_scaffold_no_longer_clears_it() {
    // The same walker is stunned instead: still walking until the next tick
    // flushes the walk, it no longer clears the ground it stands on.
    let (mut app, walker, _) = walker_across_scaffold();
    assert!(make_way::clears(app.world(), walker, Some(0)));

    utils::disable_for(&mut app, walker, 60);

    assert!(entity_def::walking(app.world(), walker));
    assert!(!make_way::clears(app.world(), walker, Some(0)));
}

#[test]
fn change_cut_short_is_judged_where_walkers_stand_now_under_continuous_model() {
    // A walker stands on the scaffold at (3, 3) through a tick, so the claim
    // plane holds (3, 3) for it; it is then at (5, 3) as the change is cut
    // short, and the kiln goes back where it stood.
    let mut app = hills_app(MovementModel::Continuous, Ramps::Short, Rise::One);
    let (kiln, _) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), kiln), "scaffold");
    let (walker, _) = utils::create_owned(&mut app, "walker", 3, 3, 0);
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .get_mut::<LocationComponent>(walker)
        .expect("a walker stands somewhere")
        .position = utils::pos(5, 3);

    utils::soft_cancel_orders(app.world_mut(), kiln);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::type_name(app.world(), kiln), "kiln");
    assert_eq!(utils::cell_of(app.world(), kiln), CellPos::new(3, 3));
}

#[test]
fn change_cut_short_with_cargo_its_form_cannot_seat_destroys_it() {
    // A recruit boards the cellar frame at (3, 3); cut short, the cellar,
    // which holds nobody, cannot take the recruit back with it, and is
    // destroyed, its frame letting the recruit out.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    utils::record_announcements(&mut app);
    let (cellar, cellar_id) = utils::create_owned(&mut app, "cellar", 3, 3, 0);
    utils::order_morph(&mut app, cellar, "furnace");
    utils::run_ticks(&mut app, 3);
    assert_eq!(entity_def::type_name(app.world(), cellar), "cellar_frame");
    let (recruit, _) = utils::create_owned(&mut app, "recruit", 4, 3, 0);
    assert_eq!(
        spawn::board(app.world_mut(), recruit, cellar),
        Boarding::Boarded
    );

    utils::soft_cancel_orders(app.world_mut(), cellar);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, cellar_id));
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied { entity, cause: DeathCause::Canceled, .. }
                    if *entity == cellar_id
            ))
    );
    assert!(!entity_def::boarded(app.world(), recruit));
    assert!(entity_def::stands_on_grid(app.world(), recruit));
}

#[test]
fn reserving_change_puts_interim_form_on_inside_its_own_reservation() {
    // A sprig claims no cells; its change through a solid bud into a solid
    // tree reserves the tree's cell, (3, 3), which the bud then stands on:
    // the reservation is the change's own, so the bud goes on and the tree
    // stands after the 10 ticks.
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (sprig, _) = utils::create_owned(&mut app, "sprig", 3, 3, 0);

    utils::order_morph(&mut app, sprig, "tree");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sprig), "bud");
    utils::run_ticks(&mut app, 10);

    assert_eq!(entity_def::type_name(app.world(), sprig), "tree");
    assert_eq!(utils::cell_of(app.world(), sprig), CellPos::new(3, 3));
}

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
    // The pod holds ground underfoot and cannot move, so its change asks the
    // ground holders; at its own berth, (4, 4) below the nest on low ground,
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
fn larva_seated_away_from_its_nest_steps_out_only_from_its_berth() {
    // A cliff nest on the plateau at (4..6, 9..11) seats its larva below the
    // cliff, at (4, 6) on low ground. Every cell is held but (3, 9) on the
    // plateau beside the nest: the larva comes out of its berth, not out of
    // the nest, and never steps up the cliff from there, so it stays seated
    // and the change does not start.
    let mut app = nest_app();
    utils::create_owned(&mut app, "cliff_nest", 4, 9, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 6));
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(3, 9));

    utils::order_morph(&mut app, larva, "drone");
    utils::run_ticks(&mut app, 1);

    assert!(entity_def::attached(app.world(), larva));
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 6));
}

#[test]
fn static_footprint_rises_over_body_seated_in_berth() {
    // The larva sits at its berth, (4, 4) below the nest, and holds no
    // ground there: a bulb, a static solid body, is raised on its cell.
    let mut app = nest_app();
    utils::create_owned(&mut app, "nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 4));

    assert!(utils::create_entity(app.world_mut(), "bulb", utils::pos(4, 4), Some(0)).is_some());
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
    assert!(entity_def::changing(app.world(), pod));
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

#[test]
fn bred_body_cut_short_goes_back_to_its_berth_whatever_lies_where_it_stood() {
    // The larva steps out of its berth at (4, 4) as a husk, and a rug, lying
    // underfoot, is laid under the husk. Cut short, the larva could not be put
    // back over the rug, but a bred body goes back to a berth, never to the
    // ground it stood on: it sits in its breeder's berth again.
    let mut app = nest_app();
    utils::create_owned(&mut app, "nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let larva = brood_of(&app, "larva");
    utils::order_morph(&mut app, larva, "glider");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::of(app.world(), larva).name, "husk");
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(4, 4));
    assert!(utils::create_entity(app.world_mut(), "rug", utils::pos(4, 4), Some(0)).is_some());

    utils::soft_cancel_orders(app.world_mut(), larva);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::of(app.world(), larva).name, "larva");
    assert!(entity_def::attached(app.world(), larva));
}

#[test]
fn bred_body_cut_short_never_stands_over_what_stood_onto_its_interim_form() {
    // The nymph steps out of its berth at (4, 4) as a cocoon, which lies
    // underfoot, and a husk stands onto it. Cut short, the nymph goes back to
    // its berth without its solid form ever taking (4, 4) from the husk.
    let mut app = nest_app();
    utils::create_owned(&mut app, "nymph_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let nymph = brood_of(&app, "nymph");
    utils::order_morph(&mut app, nymph, "glider");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::of(app.world(), nymph).name, "cocoon");
    assert_eq!(utils::cell_of(app.world(), nymph), CellPos::new(4, 4));
    let (husk, _) = utils::create_owned(&mut app, "husk", 4, 4, 0);

    utils::soft_cancel_orders(app.world_mut(), nymph);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::of(app.world(), nymph).name, "nymph");
    assert!(entity_def::attached(app.world(), nymph));
    assert_eq!(utils::cell_of(app.world(), husk), CellPos::new(4, 4));
    let layers = entity_def::of(app.world(), husk)
        .location
        .expect("a husk stands")
        .occupies();
    assert!(
        app.world()
            .resource::<Map>()
            .nav_grid()
            .is_claimed_by(layers, CellPos::new(4, 4))
    );
}

#[test]
fn bred_body_cut_short_sits_on_its_berth_point_again() {
    // The offset nest at (4, 1) seats its nymph with its middle on (5.0, 4.5),
    // its anchor (4.5, 4.0) between cells; cut short as a cocoon, the nymph
    // sits there again, not on the cell its anchor rounds to.
    let mut app = nest_app();
    utils::create_owned(&mut app, "offset_nymph_nest", 4, 1, 0);
    utils::run_ticks(&mut app, 1);
    let nymph = brood_of(&app, "nymph");
    let seated = FixedUVec2::new(utils::fixed("4.5"), utils::fixed("4.0"));
    assert_eq!(entity_def::position(app.world(), nymph), seated);
    utils::order_morph(&mut app, nymph, "glider");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::of(app.world(), nymph).name, "cocoon");

    utils::soft_cancel_orders(app.world_mut(), nymph);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::of(app.world(), nymph).name, "nymph");
    assert!(entity_def::attached(app.world(), nymph));
    assert_eq!(entity_def::position(app.world(), nymph), seated);
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
/// - `cairn_bearer` is a walker that leaves a `cairn`, a solid static 1×1 body a
///   static 1×1 `hut` also trains and a `settler` walker changes into,
///   reserving its ground;
/// - `grower` is a walker changing through a static `chrysalis` into a
///   `cairn`, and `seedling` one changing through a static `seed_pod` into a
///   static 2×2 `grove`, reserving its ground;
/// - `kiln` is a static solid body changing through an underfoot `scaffold`
///   into a `furnace`, reverting when cut short, and `cellar` one changing
///   likewise through a `cellar_frame`, a hold of one `recruit`; `sprig` a
///   static passable body changing through a solid static `bud` into a solid
///   static `tree`, reserving its ground;
/// - `beacon` is a static 1×1 body on the air layer, flat at 4, `aerie` a
///   static 2×2 one training `recruit`s and `cairn`s, `airship` a static 2×2
///   one carrying four, letting them out within one cell, and `sky_shrine` a
///   static 2×2 one that leaves a `cairn`;
/// - `wagon` is a static 1×1 ground body carrying four `recruit`s, letting
///   them out within three cells, and `burrow` a static 1×1 gateway of a
///   `warren` holding four, letting them out within three cells too;
/// - `recruit` is a walker the `hut` trains too;
/// - `invoker` casts `call_walker`, summoning a `walker` nearest any cell,
///   and `call_cairn`, summoning a `cairn` likewise;
/// - `weaver` is a walker changing, reserving its ground, into a `mat`, a
///   static 2×2 body that lies underfoot, and `netter` one changing likewise
///   into a `net`, a static 2×2 passable body;
/// - `wisp` is a walker that claims no cells;
/// - `den` is a static 2×2 body on a `deep` layer, flat at 0 under the
///   ground, training `recruit`s and `cairn`s.
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
    let deep = registry.register_layer("deep", LayerLevel::At(0));
    let delving = registry.register_domain("delving", DomainDef::new(deep, Terrains::Any, None));
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
    registry.register(
        mover("recruit", walking, ground, 1)
            .with_train_time(2)
            .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
    );
    registry.register(
        mover("weaver", walking, ground, 1).with_morphs([change("mat", MorphPlacement::Reserve)]),
    );
    let wisp = mover("wisp", walking, ground, 1);
    registry.register(wisp.with_location(walking, CellSize::ONE, Solidity::Passable, ground));
    registry.register(
        EntityTypeDef::new("mat")
            .with_location(walking, CellSize::new(2, 2), Solidity::Underfoot, ground)
            .with_pool(Pool::health(50)),
    );
    registry.register(
        mover("netter", walking, ground, 1).with_morphs([change("net", MorphPlacement::Reserve)]),
    );
    registry.register(
        EntityTypeDef::new("net")
            .with_location(walking, CellSize::new(2, 2), Solidity::Passable, ground)
            .with_pool(Pool::health(50)),
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
        EntityTypeDef::new("kiln")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_morphs([change_through(
                "scaffold",
                "furnace",
                MorphPlacement::Revalidate,
            )]),
    );
    registry.register(
        EntityTypeDef::new("scaffold")
            .with_location(walking, CellSize::ONE, Solidity::Underfoot, ground)
            .with_pool(Pool::health(50)),
    );
    registry.register(
        EntityTypeDef::new("furnace")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50)),
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
            .with_trainer(["cairn", "recruit"]),
    );
    registry.register(
        EntityTypeDef::new("aerie")
            .with_location(aloft, CellSize::new(2, 2), Solidity::Solid, air)
            .with_pool(Pool::health(50))
            .with_trainer(["recruit", "cairn"]),
    );
    registry.register(
        EntityTypeDef::new("den")
            .with_location(delving, CellSize::new(2, 2), Solidity::Solid, deep)
            .with_pool(Pool::health(50))
            .with_trainer(["recruit", "cairn"]),
    );
    registry.register(
        EntityTypeDef::new("airship")
            .with_location(aloft, CellSize::new(2, 2), Solidity::Solid, air)
            .with_pool(Pool::health(50))
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                Kinds::types(["recruit"]),
                Affiliation::Own,
                PassengerFate::Eject,
                PassengerConduct::Fight,
            ),
    );
    registry.register(
        EntityTypeDef::new("sky_shrine")
            .with_location(aloft, CellSize::new(2, 2), Solidity::Solid, air)
            .with_pool(Pool::health(50))
            .with_dying(1, utils::leaves("cairn")),
    );
    registry.register(
        EntityTypeDef::new("cellar")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_morphs([change_through(
                "cellar_frame",
                "furnace",
                MorphPlacement::Revalidate,
            )]),
    );
    registry.register(
        EntityTypeDef::new("cellar_frame")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                Kinds::types(["recruit"]),
                Affiliation::Own,
                PassengerFate::Eject,
                PassengerConduct::Shelter,
            ),
    );
    registry.register(
        EntityTypeDef::new("sprig")
            .with_location(walking, CellSize::ONE, Solidity::Passable, ground)
            .with_pool(Pool::health(50))
            .with_morphs([change_through("bud", "tree", MorphPlacement::Reserve)]),
    );
    for name in ["bud", "tree"] {
        registry.register(
            EntityTypeDef::new(name)
                .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
                .with_pool(Pool::health(50)),
        );
    }
    registry.register(
        EntityTypeDef::new("wagon")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::from_num(3))
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
            .with_transporter(
                Kinds::types(["recruit"]),
                Affiliation::Own,
                PassengerFate::Eject,
                PassengerConduct::Fight,
            ),
    );
    let warren = registry.register_network(
        "warren",
        NetworkDef::new(
            Kinds::types(["recruit"]),
            Affiliation::Own,
            4,
            PassengerFate::Destroy,
        ),
    );
    registry.register(
        EntityTypeDef::new("burrow")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_gateway(Joins::Network(warren), Doors::Both)
            .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
            .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::from_num(3))
            .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
            .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO),
    );
    let call_walker = summon_nearby(&mut registry, "call_walker", "walker");
    let call_cairn = summon_nearby(&mut registry, "call_cairn", "cairn");
    registry.register(
        EntityTypeDef::new("invoker")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(50))
            .with_skills([call_walker, call_cairn]),
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

/// Registers `name`, a skill summoning one `summoned` nearest any cell.
fn summon_nearby(registry: &mut ContentRegistry, name: &str, summoned: &str) -> SkillId {
    let entity_type = registry.type_id(summoned).expect("registered first");
    registry.register_skill(
        name,
        SkillDef {
            cooldown: 10,
            caster: SkillCaster::Entity {
                costs: Vec::new(),
                casting: Casting::Instant,
                effect: EntityCastEffect::Summon {
                    at: SummonAt::Cell {
                        reach: Reach::Wherever,
                    },
                    entity_type,
                    count: Quantity::Constant(1),
                    placement: SummonPlacement::Nearby,
                    landing: Landing::Anywhere,
                    within: Vec::new(),
                    arrives: SummonArrival::Standing,
                },
            },
            requires: Vec::new(),
        },
    )
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
///   opening with one `pod`; `nymph_nest` the same, opening with one
///   `nymph`, and `offset_nymph_nest` one whose berth lies on the edge between
///   the cells below its west and middle columns; `cliff_nest` one whose berth lies in the middle of the third
///   cell above its west column;
/// - `larva` is a passable walker that keeps its berth and changes straight
///   into a `drone`, a flier, or through a `husk`, a mover that strides
///   cliffs, into a `glider`, another flier;
/// - `nymph` is a solid walker that changes through a `cocoon`, a static
///   underfoot body, into a `glider`;
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
        EntityTypeDef::new("nymph_nest")
            .with_location(walking, CellSize::new(3, 3), Solidity::Solid, ground)
            .with_pool(Pool::health(300))
            .with_dying(2, [])
            .with_tags(["building"])
            .with_berths([("brood", BerthGroup::new([utils::berth("0.5", "3.5")], 1))])
            .with_breeder("nymph", Quantity::Constant(10), 1, 1, OrphanFate::Perish),
    );
    registry.register(
        EntityTypeDef::new("offset_nymph_nest")
            .with_location(walking, CellSize::new(3, 3), Solidity::Solid, ground)
            .with_pool(Pool::health(300))
            .with_dying(2, [])
            .with_tags(["building"])
            .with_berths([("brood", BerthGroup::new([utils::berth("1.0", "3.5")], 1))])
            .with_breeder("nymph", Quantity::Constant(10), 1, 1, OrphanFate::Perish),
    );
    registry.register(
        EntityTypeDef::new("nymph")
            .with_location(walking, CellSize::ONE, Solidity::Solid, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, [])
            .with_broodling(Attachment::new("brood", BerthStance::Still))
            .with_morphs([change_through(
                "cocoon",
                "glider",
                MorphPlacement::Revalidate,
            )]),
    );
    registry.register(
        EntityTypeDef::new("cocoon")
            .with_location(walking, CellSize::ONE, Solidity::Underfoot, ground)
            .with_pool(Pool::health(25))
            .with_dying(1, []),
    );
    registry.register(
        EntityTypeDef::new("cliff_nest")
            .with_location(walking, CellSize::new(3, 3), Solidity::Solid, ground)
            .with_pool(Pool::health(300))
            .with_dying(2, [])
            .with_tags(["building"])
            .with_berths([("brood", BerthGroup::new([utils::berth("0.5", "-2.5")], 1))])
            .with_breeder("larva", Quantity::Constant(10), 1, 1, OrphanFate::Perish),
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

/// The hills under the cell model with a kiln on (3, 3) changing into a
/// furnace, its scaffold standing from the third tick, and an own walker sent
/// from (3, 1) to (3, 7), crossing into (3, 3) on the fifth tick after: the
/// app, the walker and the kiln's id.
fn walker_across_scaffold() -> (App, Entity, SimulationId) {
    let mut app = hills_app(MovementModel::Cell, Ramps::Short, Rise::One);
    let (kiln, kiln_id) = utils::create_owned(&mut app, "kiln", 3, 3, 0);
    utils::order_morph(&mut app, kiln, "furnace");
    utils::run_ticks(&mut app, 3);
    let (walker, walker_id) = utils::create_owned(&mut app, "walker", 3, 1, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(3, 7),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::part_way("3", "2.5")
    );
    (app, walker, kiln_id)
}
